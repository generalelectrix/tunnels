use anyhow::{bail, Result};
use log::{self, error, info, warn};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender},
    time::{Duration, Instant},
};
use tunnels_lib::Timestamp;

use crate::{
    animation,
    animation_target::AnimationTarget,
    audio::{self, AudioInput},
    clock_bank::{self, ClockBank},
    control::{ControlEvent, Dispatcher},
    master_ui::{self, MasterUI},
    midi::DeviceSpec as MidiDeviceSpec,
    midi_controls::Device,
    mixer::{self, Mixer},
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
        midi_devices: Vec<MidiDeviceSpec<Device>>,
        osc_devices: Vec<OscDeviceSpec>,
        send_control_event: Sender<ControlEvent>,
        recv_control_event: Receiver<ControlEvent>,
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
            dispatcher: Dispatcher::new(
                midi_devices,
                osc_devices,
                send_control_event,
                recv_control_event,
            )?,
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
                error!("Autosave error: {e}.");
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
                warn!("{e}");
            }
        }
    }
}

#[derive(Debug)]
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
    /// Force a full UI refresh.
    UIRefresh,
}

#[derive(Debug)]
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
    use std::{
        collections::HashSet,
        sync::{mpsc::channel, Arc},
    };

    use tunnels_lib::{number::UnipolarFloat, LayerCollection, Shape};

    use super::*;
    use crate::test_mode::stress;
    use insta::assert_yaml_snapshot;

    /// Test show rendering against static test expectations.
    /// The purpose of this test is to catch accidental regressions in the
    /// tunnel state or rendering algorithm.
    #[test]
    fn test_render() -> Result<()> {
        let (send, recv) = channel();
        let mut show = Show::new(Vec::new(), Vec::new(), send, recv, None, false, None)?;

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
    fn trunc_arc_segment(seg: &mut Shape) {
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

    #[derive(Default)]
    struct RecordingMidiOutput {
        events: Vec<(crate::midi_controls::Device, crate::midi::Event)>,
    }

    impl crate::midi::MidiOutput for RecordingMidiOutput {
        fn send(&mut self, device: &crate::midi_controls::Device, event: crate::midi::Event) {
            self.events.push((*device, event));
        }
    }

    /// Regression test for all MIDI control mappings.
    ///
    /// Brute-forces every possible (Device, EventType, channel, control)
    /// combination through ControlMap::dispatch and captures the Debug
    /// output of the resulting Option<ControlMessage>.
    ///
    /// To generate/update expectations:
    ///   UPDATE_EXPECTATIONS=1 cargo test midi_interpret_regression
    #[test]
    fn midi_interpret_regression() {
        use std::io::Write;
        use crate::midi::{Event, EventType, Mapping};
        use crate::midi_controls::{ControlMap, Device};

        let expectations_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/snapshots/midi_interpret_expectations.yaml");
        let generating = std::env::var("UPDATE_EXPECTATIONS").is_ok();

        let control_map = ControlMap::new();
        let devices = [Device::AkaiApc40, Device::AkaiApc20, Device::TouchOsc, Device::BehringerCmdMM1];
        let event_types = [EventType::NoteOn, EventType::ControlChange];

        // Build a map of device_name -> full debug string of all mappings
        let mut results: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();

        for device in &devices {
            let mut output = String::new();
            for &event_type in &event_types {
                for channel in 0..16u8 {
                    for control in 0..128u8 {
                        let event = Event {
                            mapping: Mapping { event_type, channel, control },
                            value: 64,
                        };
                        let result = control_map.dispatch(*device, event);
                        if let Some(ref msg) = result {
                            output.push_str(&format!(
                                "{} {}:{} -> {:?}\n",
                                match event_type {
                                    EventType::NoteOn => "NoteOn",
                                    EventType::NoteOff => "NoteOff",
                                    EventType::ControlChange => "CC",
                                },
                                channel, control, msg
                            ));
                        }
                    }
                }
            }
            results.insert(format!("{device}"), output);
        }

        if generating {
            let yaml = serde_yaml::to_string(&results).unwrap();
            let mut file = std::fs::File::create(&expectations_path).unwrap();
            file.write_all(yaml.as_bytes()).unwrap();
            println!("Wrote interpret expectations to {}", expectations_path.display());
        } else {
            let file = std::fs::File::open(&expectations_path).unwrap_or_else(|e| {
                panic!(
                    "Missing expectations file at {}: {e}\n\
                     Run with UPDATE_EXPECTATIONS=1 to generate it.",
                    expectations_path.display()
                )
            });
            let expected: std::collections::BTreeMap<String, String> =
                serde_yaml::from_reader(file).unwrap();

            let mut failures: Vec<String> = Vec::new();
            for (device, actual) in &results {
                match expected.get(device) {
                    Some(exp) if exp != actual => {
                        failures.push(format!("{device}: output differs"));
                        // Show first differing line for debugging
                        for (i, (a, e)) in actual.lines().zip(exp.lines()).enumerate() {
                            if a != e {
                                failures.push(format!("  first diff at line {i}: got [{a}] expected [{e}]"));
                                break;
                            }
                        }
                        let a_lines = actual.lines().count();
                        let e_lines = exp.lines().count();
                        if a_lines != e_lines {
                            failures.push(format!("  line count: got {a_lines}, expected {e_lines}"));
                        }
                    }
                    None => {
                        failures.push(format!("{device}: no expectation"));
                    }
                    _ => {} // match
                }
            }

            if !failures.is_empty() {
                panic!(
                    "{} midi interpret regression failures:\n{}",
                    failures.len(),
                    failures.join("\n")
                );
            }
        }
    }

    fn all_state_changes() -> Vec<(&'static str, StateChange)> {
        use tunnels_lib::number::{BipolarFloat, UnipolarFloat};
        use tunnels_lib::RenderMode;

        let uni = UnipolarFloat::new(0.5);
        let bip = BipolarFloat::new(0.25);

        let mut changes: Vec<(&str, StateChange)> = Vec::new();

        // Tunnel state changes.
        {
            use crate::tunnel::StateChange as T;
            use crate::palette::ColorPaletteIdx;
            let mut t = |name, sc| changes.push((name, StateChange::Tunnel(sc)));
            t("tunnel/thickness", T::Thickness(uni));
            t("tunnel/size", T::Size(uni));
            t("tunnel/aspect_ratio", T::AspectRatio(uni));
            t("tunnel/color_center", T::ColorCenter(uni));
            t("tunnel/color_width", T::ColorWidth(uni));
            t("tunnel/color_spread", T::ColorSpread(uni));
            t("tunnel/color_saturation", T::ColorSaturation(uni));
            t("tunnel/palette_none", T::PaletteSelection(None));
            t("tunnel/palette_0", T::PaletteSelection(Some(ColorPaletteIdx(0))));
            t("tunnel/segments", T::Segments(4));
            t("tunnel/blacking", T::Blacking(bip));
            t("tunnel/marquee_speed", T::MarqueeSpeed(bip));
            t("tunnel/rotation_speed", T::RotationSpeed(bip));
            t("tunnel/position_x", T::PositionX(0.3));
            t("tunnel/position_y", T::PositionY(-0.2));
            t("tunnel/spin_speed", T::SpinSpeed(bip));
            t("tunnel/render_arc", T::RenderMode(RenderMode::Arc));
            t("tunnel/render_dot", T::RenderMode(RenderMode::Dot));
            t("tunnel/render_saucer", T::RenderMode(RenderMode::Saucer));
        }

        // Animation state changes.
        {
            use crate::animation::StateChange as A;
            use crate::animation::Waveform;
            use crate::clock_bank::ClockIdxExt;
            let mut a = |name, sc| changes.push((name, StateChange::Animation(sc)));
            a("anim/speed", A::Speed(bip));
            a("anim/size", A::Size(uni));
            a("anim/duty_cycle", A::DutyCycle(uni));
            a("anim/smoothing", A::Smoothing(uni));
            a("anim/waveform_sine", A::Waveform(Waveform::Sine));
            a("anim/waveform_triangle", A::Waveform(Waveform::Triangle));
            a("anim/waveform_square", A::Waveform(Waveform::Square));
            a("anim/waveform_sawtooth", A::Waveform(Waveform::Sawtooth));
            a("anim/waveform_noise", A::Waveform(Waveform::Noise));
            a("anim/waveform_constant", A::Waveform(Waveform::Constant));
            a("anim/n_periods", A::NPeriods(3));
            a("anim/pulse_on", A::Pulse(true));
            a("anim/pulse_off", A::Pulse(false));
            a("anim/invert_on", A::Invert(true));
            a("anim/standing_on", A::Standing(true));
            a("anim/clock_internal", A::ClockSource(None));
            a("anim/clock_0", A::ClockSource(Some(ClockIdxExt(0).try_into().unwrap())));
            a("anim/clock_1", A::ClockSource(Some(ClockIdxExt(1).try_into().unwrap())));
            a("anim/use_audio_size_on", A::UseAudioSize(true));
            a("anim/use_audio_speed_on", A::UseAudioSpeed(true));
        }

        // AnimationTarget state changes.
        {
            use crate::animation_target::AnimationTarget as AT;
            let mut at = |name, sc| changes.push((name, StateChange::AnimationTarget(sc)));
            at("anim_target/rotation", AT::Rotation);
            at("anim_target/thickness", AT::Thickness);
            at("anim_target/size", AT::Size);
            at("anim_target/aspect_ratio", AT::AspectRatio);
            at("anim_target/color", AT::Color);
            at("anim_target/color_spread", AT::ColorSpread);
            at("anim_target/color_saturation", AT::ColorSaturation);
            at("anim_target/marquee", AT::MarqueeRotation);
            at("anim_target/position_x", AT::PositionX);
            at("anim_target/position_y", AT::PositionY);
            at("anim_target/spin", AT::Spin);
        }

        // Mixer state changes -- test page 0 and page 1 channels.
        {
            use crate::mixer::{
                ChannelIdx, ChannelStateChange as CS, StateChange as MS, VideoChannel,
            };
            let mut m = |name, sc| changes.push((name, StateChange::Mixer(sc)));
            // Page 0, channel 0.
            m("mixer/p0_ch0_level", MS { channel: ChannelIdx(0), change: CS::Level(uni) });
            m("mixer/p0_ch0_bump_on", MS { channel: ChannelIdx(0), change: CS::Bump(true) });
            m("mixer/p0_ch0_mask_on", MS { channel: ChannelIdx(0), change: CS::Mask(true) });
            m("mixer/p0_ch0_vc0_on", MS { channel: ChannelIdx(0), change: CS::VideoChannel((VideoChannel(0), true)) });
            m("mixer/p0_ch0_contains_look", MS { channel: ChannelIdx(0), change: CS::ContainsLook(true) });
            // Page 0, channel 3.
            m("mixer/p0_ch3_level", MS { channel: ChannelIdx(3), change: CS::Level(uni) });
            // Page 1, channel 8.
            m("mixer/p1_ch8_level", MS { channel: ChannelIdx(8), change: CS::Level(uni) });
            m("mixer/p1_ch8_bump_on", MS { channel: ChannelIdx(8), change: CS::Bump(true) });
            m("mixer/p1_ch8_vc1_on", MS { channel: ChannelIdx(8), change: CS::VideoChannel((VideoChannel(1), true)) });
        }

        // Clock state changes -- test multiple clock channels.
        {
            use crate::clock::StateChange as CS;
            use crate::clock_bank::{ClockIdxExt, StateChange as CBS};
            let mut c = |name, ch: usize, sc| changes.push((name, StateChange::Clock(CBS { channel: ClockIdxExt(ch).try_into().unwrap(), change: sc })));
            c("clock/ch0_rate", 0, CS::Rate(bip));
            c("clock/ch0_rate_fine", 0, CS::RateFine(bip));
            c("clock/ch0_oneshot_on", 0, CS::OneShot(true));
            c("clock/ch0_level", 0, CS::SubmasterLevel(uni));
            c("clock/ch0_use_audio_size_on", 0, CS::UseAudioSize(true));
            c("clock/ch0_use_audio_speed_on", 0, CS::UseAudioSpeed(true));
            c("clock/ch0_ticked", 0, CS::Ticked(true));
            c("clock/ch2_rate", 2, CS::Rate(bip));
        }

        // MasterUI state changes.
        {
            use crate::master_ui::{BeamButtonState, BeamStoreState, StateChange as MU};
            use crate::beam_store::BeamStoreAddr;
            use crate::mixer::ChannelIdx;
            use crate::tunnel::AnimationIdx;
            let mut mu = |name, sc| changes.push((name, StateChange::MasterUI(sc)));
            mu("master_ui/channel_0", MU::Channel(ChannelIdx(0)));
            mu("master_ui/channel_3", MU::Channel(ChannelIdx(3)));
            mu("master_ui/channel_8", MU::Channel(ChannelIdx(8)));
            mu("master_ui/animation_0", MU::Animation(AnimationIdx(0)));
            mu("master_ui/animation_2", MU::Animation(AnimationIdx(2)));
            mu("master_ui/beam_button_empty", MU::BeamButton((BeamStoreAddr { row: 0, col: 0 }, BeamButtonState::Empty)));
            mu("master_ui/beam_button_beam", MU::BeamButton((BeamStoreAddr { row: 0, col: 0 }, BeamButtonState::Beam)));
            mu("master_ui/beam_button_look", MU::BeamButton((BeamStoreAddr { row: 0, col: 0 }, BeamButtonState::Look)));
            mu("master_ui/beam_button_p1", MU::BeamButton((BeamStoreAddr { row: 0, col: 8 }, BeamButtonState::Beam)));
            mu("master_ui/beam_store_idle", MU::BeamStoreState(BeamStoreState::Idle));
            mu("master_ui/beam_store_beam_save", MU::BeamStoreState(BeamStoreState::BeamSave));
            mu("master_ui/beam_store_look_save", MU::BeamStoreState(BeamStoreState::LookSave));
            mu("master_ui/beam_store_delete", MU::BeamStoreState(BeamStoreState::Delete));
            mu("master_ui/beam_store_look_edit", MU::BeamStoreState(BeamStoreState::LookEdit));
        }

        // Audio state changes.
        {
            use crate::audio::StateChange as AU;
            let mut au = |name, sc| changes.push((name, StateChange::Audio(sc)));
            au("audio/monitor_on", AU::Monitor(true));
            au("audio/monitor_off", AU::Monitor(false));
            au("audio/envelope", AU::EnvelopeValue(uni));
            au("audio/filter_cutoff", AU::FilterCutoff(440.0));
            au("audio/envelope_attack", AU::EnvelopeAttack(Duration::from_millis(10)));
            au("audio/envelope_release", AU::EnvelopeRelease(Duration::from_millis(50)));
            au("audio/gain", AU::Gain(2.0));
            au("audio/is_clipping", AU::IsClipping(true));
        }

        changes
    }

    /// Regression test for the MIDI emit path (StateChange -> MIDI output).
    ///
    /// For each StateChange variant, calls the current update_*_control()
    /// functions with a RecordingMidiOutput, captures the Debug output of
    /// all emitted events, and compares against a YAML expectations file.
    ///
    /// To generate/update expectations:
    ///   UPDATE_EXPECTATIONS=1 cargo test midi_emit_regression
    #[test]
    fn midi_emit_regression() {
        use std::collections::BTreeMap;
        use std::io::Write;

        use crate::midi_controls::audio::update_audio_control;

        let expectations_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/snapshots/midi_emit_expectations.yaml");
        let generating = std::env::var("UPDATE_EXPECTATIONS").is_ok();

        let state_changes = all_state_changes();
        let mut results: BTreeMap<String, String> = BTreeMap::new();

        let n_state_changes = state_changes.len();
        for (name, sc) in state_changes {
            let mut recorder = RecordingMidiOutput::default();

            // Route through the same dispatch the Dispatcher uses.
            match sc {
                StateChange::Tunnel(sc) => {
                    crate::midi_controls::tunnel::update_tunnel_control(sc, &mut recorder);
                }
                StateChange::Animation(sc) => {
                    crate::midi_controls::animation::update_animation_control(sc, &mut recorder);
                }
                StateChange::AnimationTarget(sc) => {
                    crate::midi_controls::animation_target::update_animation_target_control(sc, &mut recorder);
                }
                StateChange::Mixer(sc) => {
                    crate::midi_controls::mixer::update_mixer_control(sc, &mut recorder);
                }
                StateChange::Clock(sc) => {
                    crate::midi_controls::clock::update_clock_control(sc, &mut recorder);
                }
                StateChange::ColorPalette(_) => {
                    // No MIDI emit for color palette currently.
                }
                StateChange::MasterUI(sc) => {
                    crate::midi_controls::master_ui::update_master_ui_control(sc, &mut recorder);
                }
                StateChange::Audio(sc) => {
                    update_audio_control(sc, &mut recorder);
                }
            }

            // Format events as debug output.
            let mut output = String::new();
            for (device, event) in &recorder.events {
                output.push_str(&format!("{device}: {:?}\n", event));
            }
            results.insert(name.to_string(), output);
        }

        if generating {
            let yaml = serde_yaml::to_string(&results).unwrap();
            let mut file = std::fs::File::create(&expectations_path).unwrap();
            file.write_all(yaml.as_bytes()).unwrap();
            println!(
                "Wrote emit expectations for {} state changes to {}",
                n_state_changes,
                expectations_path.display()
            );
        } else {
            let file = std::fs::File::open(&expectations_path).unwrap_or_else(|e| {
                panic!(
                    "Missing expectations file at {}: {e}\n\
                     Run with UPDATE_EXPECTATIONS=1 to generate it.",
                    expectations_path.display()
                )
            });
            let expected: BTreeMap<String, String> =
                serde_yaml::from_reader(file).unwrap();

            let mut failures: Vec<String> = Vec::new();
            for (name, actual) in &results {
                match expected.get(name) {
                    Some(exp) if exp != actual => {
                        failures.push(format!("{name}: output differs"));
                        for (i, (a, e)) in actual.lines().zip(exp.lines()).enumerate() {
                            if a != e {
                                failures.push(format!("  first diff at line {i}: got [{a}] expected [{e}]"));
                                break;
                            }
                        }
                        let a_lines = actual.lines().count();
                        let e_lines = exp.lines().count();
                        if a_lines != e_lines {
                            failures.push(format!("  line count: got {a_lines}, expected {e_lines}"));
                        }
                    }
                    None => {
                        failures.push(format!("{name}: no expectation"));
                    }
                    _ => {} // match
                }
            }

            if !failures.is_empty() {
                panic!(
                    "{} midi emit regression failures:\n{}",
                    failures.len(),
                    failures.join("\n")
                );
            }
        }
    }
}
