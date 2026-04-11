use crate::{
    animation,
    animation_target::AnimationTarget,
    animation_visualizer::AnimationSnapshot,
    audio::{self, AudioInput, ShowEmitter},
    clock_bank::{self, ClockBank},
    clock_server::{self, ClockPublisher, SharedClockData, StaticClockBank},
    control::{ControlEvent, Dispatcher, MetaCommand, ReceivedEvent},
    gui_state::{GuiDirty, SharedGuiState},
    master_ui::{self, MasterUI},
    midi::MidiDeviceInit,
    midi_controls::Device,
    mixer::{self, Mixer},
    osc::DeviceSpec as OscDeviceSpec,
    palette::{self, ColorPalette},
    position_bank::{self, PositionBank},
    send::{Frame, start_render_service},
    test_mode::TestModeSetup,
    tunnel,
};
use anyhow::{Result, bail};
use log::{self, error, info, warn};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
};

/// How often should we autosave the show?
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(60);

pub struct Show {
    dispatcher: Dispatcher,
    audio_input: AudioInput,
    clock_publisher: Option<ClockPublisher>,
    state: ShowState,
    save_path: Option<PathBuf>,
    last_save: Option<Instant>,
    gui_state: Option<SharedGuiState>,
}

impl Show {
    #[expect(clippy::too_many_arguments)]
    /// Create a new show from the provided config.
    pub fn new(
        midi_devices: Vec<MidiDeviceInit>,
        osc_devices: Vec<OscDeviceSpec>,
        send_control_event: Sender<ControlEvent>,
        recv_control_event: Receiver<ControlEvent>,
        audio_input_device: Option<String>,
        run_clock_service: bool,
        save_path: Option<PathBuf>,
        gui_state: Option<SharedGuiState>,
    ) -> Result<Self> {
        // Determine if we need to configure a double-wide mixer for APC20 wing.
        let use_wing = midi_devices.iter().any(|init| match init {
            MidiDeviceInit::Connected(spec) => spec.device == Device::AkaiApc20,
            MidiDeviceInit::Slot { device, .. } => *device == Device::AkaiApc20,
        });

        let n_pages = if use_wing { 2 } else { 1 };

        let show = Self {
            dispatcher: Dispatcher::new(
                midi_devices,
                osc_devices,
                send_control_event,
                recv_control_event,
            )?,
            audio_input: {
                let (input, envelope_streams) = AudioInput::new(audio_input_device)?;
                if let (Some(envelope_streams), Some(gs)) = (envelope_streams, &gui_state) {
                    *gs.envelope_streams.lock().unwrap() = Some(envelope_streams);
                }
                input
            },
            clock_publisher: if run_clock_service {
                match clock_server::clock_publisher() {
                    Ok(publisher) => Some(publisher),
                    Err(e) => {
                        error!("Failed to start clock service: {e:#}");
                        None
                    }
                }
            } else {
                None
            },
            state: ShowState {
                ui: MasterUI::new(n_pages),
                mixer: Mixer::new(n_pages),
                clocks: ClockBank::default(),
                positions: PositionBank::default(),
                color_palette: ColorPalette::new(),
            },
            save_path,
            last_save: None,
            gui_state,
        };
        Ok(show)
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
        self.refresh_ui();
        self.snapshot_gui_state(GuiDirty::all());

        let mut frame_number = 0;
        let frame_sender = start_render_service()?;

        let mut last_update = Instant::now();

        loop {
            let now = Instant::now();
            let time_since_update = now - last_update;
            if time_since_update >= update_interval {
                self.update_state(time_since_update);
                last_update = now;

                if frame_sender
                    .send(Frame {
                        number: frame_number,
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
                self.send_clock_data();
                self.snapshot_animation_state();
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

    fn send_clock_data(&mut self) {
        if let Some(ref mut publisher) = self.clock_publisher {
            let data = SharedClockData {
                clock_bank: StaticClockBank(self.state.clocks.as_static()),
                audio_envelope: self.audio_input.envelope(),
            };
            if let Err(e) = publisher.send(&data) {
                error!("Failed to send clock data: {e}");
            }
        }
    }

    fn update_state(&mut self, delta_t: Duration) {
        self.audio_input
            .update_state(delta_t, &mut ShowEmitter(&mut self.dispatcher));
        let audio_envelope = self.audio_input.envelope();
        self.state
            .clocks
            .update_state(delta_t, audio_envelope, &mut self.dispatcher);
        self.state.mixer.update_state(delta_t, audio_envelope);
    }

    /// Push the full show state to all connected MIDI devices.
    fn refresh_ui(&mut self) {
        self.state.ui.emit_state(
            &mut self.state.mixer,
            &mut self.state.clocks,
            &mut self.state.color_palette,
            &mut self.audio_input,
            &mut self.dispatcher,
        );
    }

    /// Push audio subsystem state to the GUI snapshot.
    fn snapshot_audio_state(&self) {
        let Some(gui_state) = &self.gui_state else {
            return;
        };
        let ps = self.audio_input.processor_settings();
        gui_state
            .audio_state
            .store(Arc::new(crate::gui_state::AudioStateSnapshot {
                filter_cutoff_hz: ps.filter_cutoff.get(),
                envelope_attack: Duration::from_secs_f32(ps.envelope_attack.get()),
                envelope_release: Duration::from_secs_f32(ps.envelope_release.get()),
                output_smoothing: Duration::from_secs_f32(ps.output_smoothing.get()),
                gain_linear: ps.gain.get() as f64,
                auto_trim_enabled: ps.auto_trim_enabled.load(Ordering::Relaxed),
                active_band: ps.active_band.load(Ordering::Relaxed),
                norm_floor_halflife: Duration::from_secs_f32(ps.norm_floor_halflife.get()),
                norm_ceiling_halflife: Duration::from_secs_f32(ps.norm_ceiling_halflife.get()),
                norm_floor_mode: ps.norm_floor_mode.load(Ordering::Relaxed),
                norm_ceiling_mode: ps.norm_ceiling_mode.load(Ordering::Relaxed),
                update_rate: ps.get_update_rate(),
            }));
    }

    /// Push the current animation state to the GUI, if the visualizer is active.
    fn snapshot_animation_state(&mut self) {
        let Some(gui_state) = &self.gui_state else {
            return;
        };
        if !gui_state.visualizer_active.load(Ordering::Relaxed) {
            return;
        }
        let animation = self
            .state
            .ui
            .current_animation(&mut self.state.mixer)
            .map(|a| a.animation.clone())
            .unwrap_or_default();
        gui_state.animation_state.store(Arc::new(AnimationSnapshot {
            animation,
            clocks: SharedClockData {
                clock_bank: StaticClockBank(self.state.clocks.as_static()),
                audio_envelope: self.audio_input.envelope(),
            },
            fixture_count: 0,
        }));
    }

    fn service_control_event(&mut self, timeout: Duration) {
        match self.dispatcher.receive(timeout) {
            Ok(Some(ReceivedEvent::Meta(cmd, reply))) => {
                let result = self.handle_meta_command(cmd);
                if let Some(reply) = reply {
                    let _ = reply.send(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
                }
                if let Ok(dirty) = result {
                    self.snapshot_gui_state(dirty);
                }
            }
            Ok(Some(ReceivedEvent::Control(msg))) => self.state.ui.handle_control_message(
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

    fn handle_meta_command(&mut self, cmd: MetaCommand) -> Result<GuiDirty> {
        use MetaCommand::*;
        Ok(match cmd {
            RefreshUI => {
                self.refresh_ui();
                GuiDirty::all()
            }
            AddMidiDevice(spec) => {
                self.dispatcher.add_midi_device(spec)?;
                GuiDirty::MIDI_SLOTS
            }
            ClearMidiDevice { slot_name } => {
                self.dispatcher.clear_midi_device(&slot_name)?;
                GuiDirty::MIDI_SLOTS
            }
            ConnectMidiPort {
                slot_name,
                device_id,
                kind,
            } => {
                self.dispatcher
                    .connect_midi_port(&slot_name, device_id, kind)?;
                self.refresh_ui();
                GuiDirty::MIDI_SLOTS
            }
            SetAudioDevice(name) => {
                let (input, envelope_streams) = AudioInput::new(name)?;
                self.audio_input = input;
                if let (Some(envelope_streams), Some(gui_state)) = (envelope_streams, &self.gui_state) {
                    *gui_state.envelope_streams.lock().unwrap() = Some(envelope_streams);
                }
                GuiDirty::AUDIO
            }
            AudioControl(msg) => {
                self.audio_input
                    .control(msg, &mut ShowEmitter(&mut self.dispatcher));
                GuiDirty::AUDIO
            }
            StartClockService => {
                if self.clock_publisher.is_some() {
                    bail!("Clock service is already running.");
                }
                self.clock_publisher = Some(clock_server::clock_publisher()?);
                info!("Clock service started.");
                GuiDirty::CLOCK_SERVICE
            }
            StopClockService => {
                if self.clock_publisher.is_none() {
                    bail!("Clock service is not running.");
                }
                self.clock_publisher = None;
                info!("Clock service stopped.");
                GuiDirty::CLOCK_SERVICE
            }
        })
    }

    fn snapshot_gui_state(&self, dirty: GuiDirty) {
        let Some(gui_state) = &self.gui_state else {
            return;
        };
        if dirty.contains(GuiDirty::MIDI_SLOTS) {
            gui_state
                .midi_slots
                .store(Arc::new(self.dispatcher.midi_slot_statuses()));
        }
        if dirty.contains(GuiDirty::AUDIO) {
            gui_state
                .audio_device
                .store(Arc::new(self.audio_input.device_name().to_string()));
            self.snapshot_audio_state();
        }
        if dirty.contains(GuiDirty::CLOCK_SERVICE) {
            gui_state
                .clock_service_running
                .store(self.clock_publisher.is_some(), Ordering::Relaxed);
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
        sync::{Arc, mpsc::channel},
    };

    use tunnels_lib::{LayerCollection, Shape, number::UnipolarFloat};

    use super::*;
    use crate::control::{CommandClient, ControlEvent, MetaCommand, ReceivedEvent};
    use crate::test_mode::stress;
    use insta::assert_yaml_snapshot;

    /// Test show rendering against static test expectations.
    /// The purpose of this test is to catch accidental regressions in the
    /// tunnel state or rendering algorithm.
    #[test]
    fn test_render() -> Result<()> {
        use crate::midi::default_midi_slots;
        let (send, recv) = channel();
        let mut show = Show::new(
            default_midi_slots(),
            Vec::new(),
            send,
            recv,
            None,
            false,
            None,
            None,
        )?;

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
        seg.extent_x = trunc_f64(seg.extent_x);
        seg.extent_y = trunc_f64(seg.extent_y);
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
    /// combination through Device::interpret and captures the Debug
    /// output of the resulting Option<ControlMessage>.
    ///
    /// To generate/update expectations:
    ///   UPDATE_EXPECTATIONS=1 cargo test midi_interpret_regression
    #[test]
    fn midi_interpret_regression() {
        use crate::midi::{Event, EventType, Mapping};
        use crate::midi_controls::{Device, MidiHandler};
        use std::io::Write;

        let expectations_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/snapshots/midi_interpret_expectations.yaml");
        let generating = std::env::var("UPDATE_EXPECTATIONS").is_ok();

        let devices = [
            Device::AkaiApc40,
            Device::AkaiApc20,
            Device::TouchOsc,
            Device::BehringerCmdMM1,
        ];
        let event_types = [
            EventType::NoteOn,
            EventType::NoteOff,
            EventType::ControlChange,
        ];

        // Build a map of device_name -> full debug string of all mappings
        let mut results: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();

        for device in &devices {
            let mut output = String::new();
            for &event_type in &event_types {
                for channel in 0..16u8 {
                    for control in 0..128u8 {
                        let event = Event {
                            mapping: Mapping {
                                event_type,
                                channel,
                                control,
                            },
                            value: 64,
                        };
                        let result = device.interpret(&event);
                        if let Some(ref msg) = result {
                            output.push_str(&format!(
                                "{} {}:{} -> {:?}\n",
                                match event_type {
                                    EventType::NoteOn => "NoteOn",
                                    EventType::NoteOff => "NoteOff",
                                    EventType::ControlChange => "CC",
                                },
                                channel,
                                control,
                                msg
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
            println!(
                "Wrote interpret expectations to {}",
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
                                failures.push(format!(
                                    "  first diff at line {i}: got [{a}] expected [{e}]"
                                ));
                                break;
                            }
                        }
                        let a_lines = actual.lines().count();
                        let e_lines = exp.lines().count();
                        if a_lines != e_lines {
                            failures
                                .push(format!("  line count: got {a_lines}, expected {e_lines}"));
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
        use tunnels_lib::{PathShape, RenderMode};

        let uni = UnipolarFloat::new(0.5);
        let bip = BipolarFloat::new(0.25);

        let mut changes: Vec<(&str, StateChange)> = Vec::new();

        // Tunnel state changes.
        {
            use crate::palette::ColorPaletteIdx;
            use crate::tunnel::StateChange as T;
            let mut t = |name, sc| changes.push((name, StateChange::Tunnel(sc)));
            t("tunnel/thickness", T::Thickness(uni));
            t("tunnel/size", T::Size(uni));
            t("tunnel/aspect_ratio", T::AspectRatio(uni));
            t("tunnel/color_center", T::ColorCenter(uni));
            t("tunnel/color_width", T::ColorWidth(uni));
            t("tunnel/color_spread", T::ColorSpread(uni));
            t("tunnel/color_saturation", T::ColorSaturation(uni));
            t("tunnel/palette_none", T::PaletteSelection(None));
            t(
                "tunnel/palette_0",
                T::PaletteSelection(Some(ColorPaletteIdx(0))),
            );
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
            t("tunnel/path_ellipse", T::PathShape(PathShape::Ellipse));
            t("tunnel/path_line", T::PathShape(PathShape::Line));
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
            a(
                "anim/clock_0",
                A::ClockSource(Some(ClockIdxExt(0).try_into().unwrap())),
            );
            a(
                "anim/clock_1",
                A::ClockSource(Some(ClockIdxExt(1).try_into().unwrap())),
            );
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
            m(
                "mixer/p0_ch0_level",
                MS {
                    channel: ChannelIdx(0),
                    change: CS::Level(uni),
                },
            );
            m(
                "mixer/p0_ch0_bump_on",
                MS {
                    channel: ChannelIdx(0),
                    change: CS::Bump(true),
                },
            );
            m(
                "mixer/p0_ch0_mask_on",
                MS {
                    channel: ChannelIdx(0),
                    change: CS::Mask(true),
                },
            );
            m(
                "mixer/p0_ch0_vc0_on",
                MS {
                    channel: ChannelIdx(0),
                    change: CS::VideoChannel((VideoChannel(0), true)),
                },
            );
            m(
                "mixer/p0_ch0_contains_look",
                MS {
                    channel: ChannelIdx(0),
                    change: CS::ContainsLook(true),
                },
            );
            // Page 0, channel 3.
            m(
                "mixer/p0_ch3_level",
                MS {
                    channel: ChannelIdx(3),
                    change: CS::Level(uni),
                },
            );
            // Page 1, channel 8.
            m(
                "mixer/p1_ch8_level",
                MS {
                    channel: ChannelIdx(8),
                    change: CS::Level(uni),
                },
            );
            m(
                "mixer/p1_ch8_bump_on",
                MS {
                    channel: ChannelIdx(8),
                    change: CS::Bump(true),
                },
            );
            m(
                "mixer/p1_ch8_vc1_on",
                MS {
                    channel: ChannelIdx(8),
                    change: CS::VideoChannel((VideoChannel(1), true)),
                },
            );
        }

        // Clock state changes -- test multiple clock channels.
        {
            use crate::clock::StateChange as CS;
            use crate::clock_bank::{ClockIdxExt, StateChange as CBS};
            let mut c = |name, ch: usize, sc| {
                changes.push((
                    name,
                    StateChange::Clock(CBS {
                        channel: ClockIdxExt(ch).try_into().unwrap(),
                        change: sc,
                    }),
                ))
            };
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
            use crate::beam_store::BeamStoreAddr;
            use crate::master_ui::{BeamButtonState, BeamStoreState, StateChange as MU};
            use crate::mixer::ChannelIdx;
            use crate::tunnel::AnimationIdx;
            let mut mu = |name, sc| changes.push((name, StateChange::MasterUI(sc)));
            mu("master_ui/channel_0", MU::Channel(ChannelIdx(0)));
            mu("master_ui/channel_3", MU::Channel(ChannelIdx(3)));
            mu("master_ui/channel_8", MU::Channel(ChannelIdx(8)));
            mu("master_ui/animation_0", MU::Animation(AnimationIdx(0)));
            mu("master_ui/animation_2", MU::Animation(AnimationIdx(2)));
            mu(
                "master_ui/beam_button_empty",
                MU::BeamButton((BeamStoreAddr { row: 0, col: 0 }, BeamButtonState::Empty)),
            );
            mu(
                "master_ui/beam_button_beam",
                MU::BeamButton((BeamStoreAddr { row: 0, col: 0 }, BeamButtonState::Beam)),
            );
            mu(
                "master_ui/beam_button_look",
                MU::BeamButton((BeamStoreAddr { row: 0, col: 0 }, BeamButtonState::Look)),
            );
            mu(
                "master_ui/beam_button_p1",
                MU::BeamButton((BeamStoreAddr { row: 0, col: 8 }, BeamButtonState::Beam)),
            );
            mu(
                "master_ui/beam_store_idle",
                MU::BeamStoreState(BeamStoreState::Idle),
            );
            mu(
                "master_ui/beam_store_beam_save",
                MU::BeamStoreState(BeamStoreState::BeamSave),
            );
            mu(
                "master_ui/beam_store_look_save",
                MU::BeamStoreState(BeamStoreState::LookSave),
            );
            mu(
                "master_ui/beam_store_delete",
                MU::BeamStoreState(BeamStoreState::Delete),
            );
            mu(
                "master_ui/beam_store_look_edit",
                MU::BeamStoreState(BeamStoreState::LookEdit),
            );
        }

        // Audio state changes.
        {
            use crate::audio::StateChange as AU;
            let mut au = |name, sc| changes.push((name, StateChange::Audio(sc)));
            au("audio/monitor_on", AU::Monitor(true));
            au("audio/monitor_off", AU::Monitor(false));
            au("audio/envelope", AU::EnvelopeValue(uni));
            au("audio/filter_cutoff", AU::FilterCutoff(440.0));
            au(
                "audio/envelope_attack",
                AU::EnvelopeAttack(Duration::from_millis(10)),
            );
            au(
                "audio/envelope_release",
                AU::EnvelopeRelease(Duration::from_millis(50)),
            );
            au(
                "audio/output_smoothing",
                AU::OutputSmoothing(Duration::from_millis(8)),
            );
            au("audio/auto_trim_enabled", AU::AutoTrimEnabled(true));
            au("audio/input_gain", AU::InputGain(2.0));
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
                    crate::midi_controls::animation_target::update_animation_target_control(
                        sc,
                        &mut recorder,
                    );
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
            let expected: BTreeMap<String, String> = serde_yaml::from_reader(file).unwrap();

            let mut failures: Vec<String> = Vec::new();
            for (name, actual) in &results {
                match expected.get(name) {
                    Some(exp) if exp != actual => {
                        failures.push(format!("{name}: output differs"));
                        for (i, (a, e)) in actual.lines().zip(exp.lines()).enumerate() {
                            if a != e {
                                failures.push(format!(
                                    "  first diff at line {i}: got [{a}] expected [{e}]"
                                ));
                                break;
                            }
                        }
                        let a_lines = actual.lines().count();
                        let e_lines = exp.lines().count();
                        if a_lines != e_lines {
                            failures
                                .push(format!("  line count: got {a_lines}, expected {e_lines}"));
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

    impl Show {
        /// Create a minimal test Show with default MIDI slots but no hardware.
        fn test_new() -> (Self, Sender<ControlEvent>) {
            use crate::midi::default_midi_slots;
            let (send, recv) = channel();
            let show = Show::new(
                default_midi_slots(),
                Vec::new(),
                send.clone(),
                recv,
                None,
                false,
                None,
                None,
            )
            .unwrap();
            (show, send)
        }
    }

    /// Stand up a test Show on a background thread, return a CommandClient.
    fn test_show_client() -> CommandClient {
        let (client_tx, client_rx) = std::sync::mpsc::sync_channel(0);
        std::thread::spawn(move || {
            let (mut show, send) = Show::test_new();
            client_tx.send(CommandClient::new(send)).unwrap();
            // Process events until the client disconnects.
            loop {
                match show.dispatcher.receive(Duration::from_millis(50)) {
                    Ok(Some(ReceivedEvent::Meta(cmd, reply))) => {
                        let result = show.handle_meta_command(cmd);
                        if let Some(reply) = reply {
                            let _ = reply
                                .send(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
                        }
                        if let Ok(dirty) = result {
                            show.snapshot_gui_state(dirty);
                        }
                    }
                    Ok(Some(ReceivedEvent::Control(msg))) => {
                        show.state.ui.handle_control_message(
                            msg,
                            &mut show.state.mixer,
                            &mut show.state.clocks,
                            &mut show.state.color_palette,
                            &mut show.state.positions,
                            &mut show.audio_input,
                            &mut show.dispatcher,
                        );
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        });
        client_rx.recv().unwrap()
    }

    #[test]
    fn meta_refresh_ui() {
        let client = test_show_client();
        client.send_command(MetaCommand::RefreshUI).unwrap();
    }

    #[test]
    fn meta_set_audio_device_offline() {
        let client = test_show_client();
        client
            .send_command(MetaCommand::SetAudioDevice(None))
            .unwrap();
    }

    #[test]
    fn meta_connect_midi_port_unknown_slot() {
        let client = test_show_client();
        let err = client
            .send_command(MetaCommand::ConnectMidiPort {
                slot_name: "nonexistent".to_string(),
                device_id: midi_harness::DeviceId("fake".into()),
                kind: midi_harness::DeviceKind::Input,
            })
            .unwrap_err();
        assert!(err.to_string().contains("unknown device slot"), "{err}");
    }

    #[test]
    fn meta_clear_midi_device_unknown_slot() {
        let client = test_show_client();
        let err = client
            .send_command(MetaCommand::ClearMidiDevice {
                slot_name: "nonexistent".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("unknown device slot"), "{err}");
    }

    #[test]
    fn meta_round_trip_response() {
        let client = test_show_client();
        // Valid command should succeed.
        client.send_command(MetaCommand::RefreshUI).unwrap();
        // Invalid command should return the right error.
        let err = client
            .send_command(MetaCommand::ClearMidiDevice {
                slot_name: "nope".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("unknown device slot"), "{err}");
        // Should still work after error.
        client.send_command(MetaCommand::RefreshUI).unwrap();
    }
}
