use anyhow::{bail, Result};
use log::{self, error, info, warn};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tunnels_lib::Timestamp;

use crate::{
    animation,
    animation_target::AnimationTarget,
    audio::{self, AudioInput},
    clock_bank::{self, ClockBank},
    control::Dispatcher,
    master_ui,
    master_ui::MasterUI,
    midi::DeviceSpec as MidiDeviceSpec,
    midi_controls::Device,
    mixer,
    mixer::Mixer,
    osc::DeviceSpec as OscDeviceSpec,
    palette::{self, ColorPalette},
    position_bank::{self, PositionBank},
    send::{start_render_service, Frame},
    test_mode::TestModeSetup,
    timesync::TimesyncServer,
    tunnel,
};

/// How often should we autosave the show?
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(60);

pub struct Show {
    dispatcher: Dispatcher,
    audio_input: AudioInput,
    run_clock_service: bool,
    state: ShowState,
    save_path: Option<PathBuf>,
    last_save: Option<Instant>,
}

impl Show {
    /// Create a new show from the provided config.
    pub fn new(
        midi_devices: Vec<MidiDeviceSpec>,
        osc_devices: Vec<OscDeviceSpec>,
        audio_input_device: Option<String>,
        run_clock_service: bool,
        save_path: Option<PathBuf>,
    ) -> Result<Self> {
        // Determine if we need to configure a double-wide mixer for APC20 wing.
        let use_wing = midi_devices
            .iter()
            .any(|spec| spec.device == Device::AkaiApc20);

        let n_pages = if use_wing { 2 } else { 1 };

        Ok(Self {
            dispatcher: Dispatcher::new(midi_devices, osc_devices)?,
            audio_input: AudioInput::new(audio_input_device)?,
            run_clock_service,
            state: ShowState {
                ui: MasterUI::new(n_pages),
                mixer: Mixer::new(n_pages),
                clocks: ClockBank::default(),
                positions: PositionBank::default(),
                color_palette: ColorPalette::new(),
            },
            save_path,
            last_save: None,
        })
    }

    /// Load the saved show at file into self.
    /// Return an error if the dimensions of the loaded data don't match the
    /// current show.
    pub fn load(&mut self, path: &Path) -> Result<()> {
        let file = File::open(path)?;
        let loaded_state = ShowState::deserialize(&mut Deserializer::new(file))?;
        if loaded_state.mixer.channel_count() != self.state.mixer.channel_count() {
            bail!(
                "Mixer size mismatch. Loaded: {}, show: {}.",
                loaded_state.mixer.channel_count(),
                self.state.mixer.channel_count()
            );
        }
        if loaded_state.ui.n_pages() != self.state.ui.n_pages() {
            bail!(
                "UI page count mismatch. Loaded: {}, show: {}.",
                loaded_state.ui.n_pages(),
                self.state.ui.n_pages()
            );
        }
        self.state = loaded_state;
        Ok(())
    }

    /// Save the show into the provided file.
    fn save(&self, path: &Path) -> Result<()> {
        let mut file = File::create(path)?;
        self.state
            .serialize(&mut Serializer::new(BufWriter::new(&mut file)))?;
        Ok(())
    }

    /// If a save path is set and we're due to save, save the show.
    fn autosave(&mut self) -> Result<()> {
        if let Some(path) = &self.save_path {
            let now = Instant::now();
            let should_save = match self.last_save {
                Some(t) => (t + AUTOSAVE_INTERVAL) <= now,
                None => true,
            };
            if should_save {
                info!("Autosaving.");
                let result = self.save(path);
                if result.is_ok() {
                    self.last_save = Some(now);
                }
                return result;
            }
        }
        Ok(())
    }

    /// Set up the show in a test mode, defined by the provided setup function.
    pub fn test_mode(&mut self, setup: TestModeSetup) {
        let channel_count = self.state.mixer.channels().count();
        self.state
            .mixer
            .channels()
            .enumerate()
            .for_each(|(i, chan)| setup(channel_count, i, chan));
    }

    /// Run the show in the current thread.
    pub fn run(&mut self, update_interval: Duration) -> Result<()> {
        info!("Show is starting.");

        // Emit initial UI state.
        self.state.ui.emit_state(
            &mut self.state.mixer,
            &mut self.state.clocks,
            &mut self.state.color_palette,
            &mut self.audio_input,
            &mut self.dispatcher,
        );

        let mut frame_number = 0;
        let ctx = zmq::Context::new();
        let start = Instant::now();

        let _timesync = TimesyncServer::start(&ctx, start)?;
        let frame_sender = start_render_service(&ctx, self.run_clock_service)?;

        let mut last_update = start;

        loop {
            let now = Instant::now();
            let time_since_update = now - last_update;
            if time_since_update >= update_interval {
                self.update_state(time_since_update);
                last_update = now;
                let timestamp = Timestamp::since(start);

                if frame_sender
                    .send(Frame {
                        number: frame_number,
                        timestamp,
                        mixer: self.state.mixer.clone(),
                        clocks: self.state.clocks.clone(),
                        color_palette: self.state.color_palette.clone(),
                        positions: self.state.positions.clone(),
                        audio_envelope: self.audio_input.envelope(),
                    })
                    .is_err()
                {
                    bail!("Render server hung up.  Aborting show.");
                }
                frame_number += 1;
            }

            // Consider autosaving the show.
            if let Err(e) = self.autosave() {
                error!("Autosave error: {}.", e);
            }

            // Process a control event for a fraction of the time between now
            // and when we need to update state again.
            if let Some(time_to_next_update) =
                (last_update + update_interval).checked_duration_since(Instant::now())
            {
                // Use 80% of the time remaining to potentially process a
                // control event.
                let timeout = time_to_next_update.mul_f64(0.8);
                self.service_control_event(timeout);
            }
        }
    }

    fn update_state(&mut self, delta_t: Duration) {
        self.audio_input.update_state(delta_t, &mut self.dispatcher);
        let audio_envelope = self.audio_input.envelope();
        self.state
            .clocks
            .update_state(delta_t, audio_envelope, &mut self.dispatcher);
        self.state.mixer.update_state(delta_t, audio_envelope);
    }

    fn service_control_event(&mut self, timeout: Duration) {
        match self.dispatcher.receive(timeout) {
            Ok(Some(msg)) => self.state.ui.handle_control_message(
                msg,
                &mut self.state.mixer,
                &mut self.state.clocks,
                &mut self.state.color_palette,
                &mut self.state.positions,
                &mut self.audio_input,
                &mut self.dispatcher,
            ),
            Ok(None) => (),
            Err(e) => {
                warn!("{}", e);
            }
        }
    }
}

pub enum ControlMessage {
    Tunnel(tunnel::ControlMessage),
    AnimationTarget(AnimationTarget),
    Animation(animation::ControlMessage),
    Mixer(mixer::ControlMessage),
    Clock(clock_bank::ControlMessage),
    ColorPalette(palette::ControlMessage),
    Position(position_bank::ControlMessage),
    Audio(audio::ControlMessage),
    MasterUI(master_ui::ControlMessage),
}

pub enum StateChange {
    Tunnel(tunnel::StateChange),
    Animation(animation::StateChange),
    AnimationTarget(AnimationTarget),
    Mixer(mixer::StateChange),
    Clock(clock_bank::StateChange),
    ColorPalette(palette::StateChange),
    Audio(audio::StateChange),
    MasterUI(master_ui::StateChange),
}

/// Proxy type for easily saving and loading show state.
#[derive(Serialize, Deserialize)]
pub struct ShowState {
    pub ui: MasterUI,
    pub mixer: Mixer,
    pub clocks: ClockBank,
    pub positions: PositionBank,
    pub color_palette: ColorPalette,
}

#[cfg(test)]
mod test {
    use std::{collections::HashSet, sync::Arc};

    use tunnels_lib::{number::UnipolarFloat, ArcSegment, LayerCollection};

    use super::*;
    use crate::test_mode::stress;
    use insta::assert_yaml_snapshot;

    /// Test show rendering against static test expectations.
    /// The purpose of this test is to catch accidental regressions in the
    /// tunnel state or rendering algorithm.
    #[test]
    fn test_render() -> Result<()> {
        let mut show = Show::new(Vec::new(), Vec::new(), None, false, None)?;

        show.test_mode(stress);

        assert_yaml_snapshot!("before_evolution", check_render(&show, 1));

        // Evolve by one timestep.
        show.update_state(Duration::from_micros(16667));

        assert_yaml_snapshot!(
            "after_evolution",
            check_render(&show, show.state.mixer.channel_count())
        );
        Ok(())
    }

    /// Render the state of the show with some assertions on structure.
    fn check_render(show: &Show, unique_beam_count: usize) -> LayerCollection {
        let video_feeds = show.state.mixer.render(
            &show.state.clocks,
            &show.state.color_palette,
            &show.state.positions,
            UnipolarFloat::ZERO,
        );

        // Should have the expected number of video channels.
        assert_eq!(Mixer::N_VIDEO_CHANNELS, video_feeds.len());

        // Channel 0 should contain data, but none of the others.
        for (i, chan) in video_feeds.iter().enumerate() {
            if i == 0 {
                assert!(!chan.is_empty());
            } else {
                assert_eq!(0, chan.len());
            }
        }

        let mut first_channel = video_feeds.into_iter().next().unwrap();

        for beam in first_channel.iter_mut() {
            for seg in Arc::get_mut(beam).unwrap().iter_mut() {
                trunc_arc_segment(seg);
            }
        }

        let beam_hashes: HashSet<_> = first_channel.iter().collect();
        assert_eq!(beam_hashes.len(), unique_beam_count);
        first_channel
    }

    /// Truncate the values in an arc segment to a reasonable precision.
    /// This should avoid very minor platform-dependent floating point differences.
    fn trunc_arc_segment(seg: &mut ArcSegment) {
        seg.level = trunc_f64(seg.level);
        seg.thickness = trunc_f64(seg.thickness);
        seg.hue = trunc_f64(seg.hue);
        seg.sat = trunc_f64(seg.sat);
        seg.val = trunc_f64(seg.val);
        seg.rad_x = trunc_f64(seg.rad_x);
        seg.rad_y = trunc_f64(seg.rad_y);
        seg.start = trunc_f64(seg.start);
        seg.stop = trunc_f64(seg.stop);
        seg.rot_angle = trunc_f64(seg.rot_angle);
    }

    /// Truncate a unit-float to 15 decimal places.
    fn trunc_f64(v: f64) -> f64 {
        (v * 1_000_000_000_000_000.).trunc() / 1_000_000_000_000_000.
    }
}
