//! Audio input and envelope extraction: the four role envelopes (see
//! [`roles`]), derived on the audio thread from a resonator bank and read by
//! the show.
//!
//! The chain is pinned by `tests/envelope_golden.rs` (response shapes) and
//! `tests/music_convergence.rs` (long-term stability). When either fails, or
//! before changing the processor, run the characterisation harness:
//! `cargo run -p tunnels_audio --release --example envelope_suite -- <dir>`
//! records every role and the level roles' stages per buffer for a suite of synthetic
//! waveforms and prints their metrics; `--music tests/data/<clip> --loops N`
//! does the same for a looped real-music clip.

pub mod bank;
pub mod processor;
pub mod reconnect;
pub mod ring_buffer;
pub mod roles;

use anyhow::{Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{info, warn};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;
use tunnels_lib::prompt::{prompt_bool, prompt_indexed_value};

pub use self::processor::UpdateRate;
use self::processor::{ProcessorSettings, ProcessorSettingsInner};
use self::reconnect::ReconnectingInput;
pub use self::ring_buffer::EnvelopeStream;
use self::roles::{NUM_ROLES, Role};

/// Device name used when no audio device is connected.
pub const OFFLINE_DEVICE_NAME: &str = "Offline";

/// Envelope data streams from the audio thread, one per role in
/// [`Role::ALL`] order, bundled with the callback rate.
pub struct EnvelopeStreams {
    pub streams: [EnvelopeStream; NUM_ROLES],
    pub update_rate: UpdateRate,
}

/// A flat, read-only view of the audio input's current parameter state.
#[derive(Debug, Clone)]
pub struct AudioSnapshot {
    pub device_name: String,
    pub envelope_attack: Duration,
    pub envelope_release: Duration,
    pub output_smoothing: Duration,
    pub active_role: Role,
    pub norm_floor_halflife: Duration,
    pub norm_ceiling_halflife: Duration,
    /// The gain the automatic input trim is applying, in dB, to the nearest
    /// [`TRIM_DISPLAY_STEP_DB`].
    pub trim_db: f32,
}

/// The resolution input trim is reported at. The trim moves slowly and
/// continuously, so a coarse step keeps it from republishing every frame.
pub const TRIM_DISPLAY_STEP_DB: f32 = 0.5;

/// A trim gain in dB, to the nearest [`TRIM_DISPLAY_STEP_DB`].
fn displayed_trim_db(gain: f32) -> f32 {
    (20.0 * gain.log10() / TRIM_DISPLAY_STEP_DB).round() * TRIM_DISPLAY_STEP_DB
}

impl AudioSnapshot {
    /// The parameter state a settings handle currently holds.
    fn read(device_name: &str, ps: &ProcessorSettingsInner) -> Self {
        Self {
            device_name: device_name.to_string(),
            envelope_attack: Duration::from_secs_f32(ps.envelope_attack.get()),
            envelope_release: Duration::from_secs_f32(ps.envelope_release.get()),
            output_smoothing: Duration::from_secs_f32(ps.output_smoothing.get()),
            active_role: Role::from_index(ps.active_role.load(Ordering::Relaxed) as usize)
                .unwrap_or(ProcessorSettingsInner::DEFAULT_ROLE),
            norm_floor_halflife: Duration::from_secs_f32(ps.norm_floor_halflife.get()),
            norm_ceiling_halflife: Duration::from_secs_f32(ps.norm_ceiling_halflife.get()),
            trim_db: displayed_trim_db(ps.trim_gain.get()),
        }
    }
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self::read(OFFLINE_DEVICE_NAME, &ProcessorSettingsInner::default())
    }
}

pub struct AudioInput {
    _input: Option<ReconnectingInput>,
    processor_settings: ProcessorSettings,
    /// Locally-stored value of the envelope.
    envelope_value: UnipolarFloat,
    /// Should we send monitor updates?
    monitor: bool,
    /// How long has it been since we last updated the monitor?
    monitor_update_age: Duration,
    /// Decides when clipping is worth a warning.
    clip_reporter: ClipReporter,
    /// The trim last reported by [`AudioInput::trim_moved`], in dB.
    reported_trim_db: f32,
    /// Name of the audio device, or "Offline" if no device is connected.
    device_name: String,
}

/// When clipping on the input is worth saying.
///
/// Said once when it starts and then only occasionally, since it is a
/// condition rather than an event.
struct ClipReporter {
    /// The processor's clip count when last read.
    clips: u32,
    /// How long ago clipping was last reported. Starts a full interval old so
    /// the first clipping is reported as it happens.
    since_report: Duration,
}

impl Default for ClipReporter {
    fn default() -> Self {
        Self {
            clips: 0,
            since_report: Self::INTERVAL,
        }
    }
}

impl ClipReporter {
    /// How often to say that the input is still clipping, for as long as it
    /// keeps clipping.
    const INTERVAL: Duration = Duration::from_secs(10);

    /// Read the processor's running clip count after `delta_t`, returning the
    /// number of new clip runs when they should be reported.
    fn update(&mut self, clips: u32, delta_t: Duration) -> Option<u32> {
        self.since_report = self.since_report.saturating_add(delta_t);
        let new_clips = clips.wrapping_sub(self.clips);
        self.clips = clips;
        if new_clips == 0 || self.since_report < Self::INTERVAL {
            return None;
        }
        self.since_report = Duration::ZERO;
        Some(new_clips)
    }
}

impl AudioInput {
    /// Update the monitor at about 60 fps.
    const MONITOR_UPDATE_INTERVAL: Duration = Duration::from_micros(16_667);
    /// Get the names of all available input audio devices.
    pub fn devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let devices = host.input_devices()?;
        let device_names = devices.map(|d| d.name().unwrap_or_else(|e| e.to_string()));
        Ok(device_names.collect())
    }

    fn offline() -> Self {
        Self {
            _input: None,
            processor_settings: ProcessorSettings::default(),
            envelope_value: UnipolarFloat::ZERO,
            monitor: false,
            monitor_update_age: Duration::ZERO,
            clip_reporter: ClipReporter::default(),
            reported_trim_db: 0.0,
            device_name: OFFLINE_DEVICE_NAME.to_string(),
        }
    }

    /// Open an audio input device. On every successful open — initial and
    /// each subsequent reconnect — a fresh `EnvelopeStreams` bundle is sent
    /// on `envelope_tx`.
    pub fn new(device_name: Option<String>, envelope_tx: Sender<EnvelopeStreams>) -> Result<Self> {
        let device_name = match device_name {
            None => return Ok(Self::offline()),
            Some(d) => d,
        };

        info!("Using audio input device {device_name}.");

        let processor_settings = ProcessorSettings::default();
        let input =
            ReconnectingInput::new(device_name.clone(), processor_settings.clone(), envelope_tx)?;

        Ok(Self {
            _input: Some(input),
            processor_settings,
            envelope_value: UnipolarFloat::ZERO,
            monitor: false,
            monitor_update_age: Duration::ZERO,
            clip_reporter: ClipReporter::default(),
            reported_trim_db: 0.0,
            device_name,
        })
    }

    /// Return the processor settings handle (for visualization tools).
    pub fn processor_settings(&self) -> &ProcessorSettings {
        &self.processor_settings
    }

    /// Read the current audio parameter state.
    ///
    /// Individual fields are self-consistent; the overall struct is not a
    /// torn-free snapshot across all fields.
    pub fn snapshot(&self) -> AudioSnapshot {
        AudioSnapshot::read(&self.device_name, &self.processor_settings)
    }

    /// Whether the input trim has moved by a displayed step since this last
    /// returned true.
    pub fn trim_moved(&mut self) -> bool {
        let trim_db = displayed_trim_db(self.processor_settings.trim_gain.get());
        let moved = trim_db != self.reported_trim_db;
        self.reported_trim_db = trim_db;
        moved
    }

    /// Update the state of audio control.
    pub fn update_state<E: EmitStateChange>(&mut self, delta_t: Duration, emitter: &mut E) {
        self.report_input_clipping(delta_t);
        let envelope = self.processor_settings.envelope.get() as f64;
        self.envelope_value = UnipolarFloat::new(envelope);
        if self.monitor {
            self.monitor_update_age += delta_t;
            if self.monitor_update_age >= Self::MONITOR_UPDATE_INTERVAL {
                self.monitor_update_age = Duration::ZERO;
                emitter.emit_audio_state_change(StateChange::EnvelopeValue(self.envelope_value));
            }
        }
    }

    /// Warn when the input is arriving already clipped: the converter has
    /// thrown that signal away, so the only fix is turning something down
    /// ahead of us.
    fn report_input_clipping(&mut self, delta_t: Duration) {
        let clips = self.processor_settings.input_clips.load(Ordering::Relaxed);
        if let Some(new_clips) = self.clip_reporter.update(clips, delta_t) {
            warn!(
                "Audio input is clipping ({new_clips} runs at full scale). \
                 Turn down the interface's input gain or the feed driving it."
            );
        }
    }

    /// Emit the current value of all controllable state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_audio_state_change(EnvelopeValue(self.envelope_value));
        emitter.emit_audio_state_change(Monitor(self.monitor));
        emitter.emit_audio_state_change(EnvelopeAttack(Duration::from_secs_f32(
            self.processor_settings.envelope_attack.get(),
        )));
        emitter.emit_audio_state_change(EnvelopeRelease(Duration::from_secs_f32(
            self.processor_settings.envelope_release.get(),
        )));
        emitter.emit_audio_state_change(OutputSmoothing(Duration::from_secs_f32(
            self.processor_settings.output_smoothing.get(),
        )));
        emitter.emit_audio_state_change(ActiveRole(self.snapshot().active_role));
        emitter.emit_audio_state_change(NormFloorHalflife(Duration::from_secs_f32(
            self.processor_settings.norm_floor_halflife.get(),
        )));
        emitter.emit_audio_state_change(NormCeilingHalflife(Duration::from_secs_f32(
            self.processor_settings.norm_ceiling_halflife.get(),
        )));
    }

    /// Handle a control event.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            ToggleMonitor => {
                self.monitor = !self.monitor;
                emitter.emit_audio_state_change(StateChange::Monitor(self.monitor));
                if !self.monitor {
                    emitter
                        .emit_audio_state_change(StateChange::EnvelopeValue(UnipolarFloat::ZERO));
                }
            }
            ResetParameters => {
                self.processor_settings.reset_defaults();
                self.emit_state(emitter);
            }
            Set(sc) => self.handle_state_change(sc, emitter),
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            EnvelopeValue(_) => return, // output only
            Monitor(v) => self.monitor = v,
            EnvelopeAttack(v) => self.processor_settings.envelope_attack.set(v.as_secs_f32()),
            EnvelopeRelease(v) => self
                .processor_settings
                .envelope_release
                .set(v.as_secs_f32()),
            OutputSmoothing(v) => self
                .processor_settings
                .output_smoothing
                .set(v.as_secs_f32()),
            ActiveRole(role) => {
                self.processor_settings
                    .active_role
                    .store(role.index() as u32, Ordering::Relaxed);
            }
            NormFloorHalflife(v) => {
                self.processor_settings
                    .norm_floor_halflife
                    .set(v.as_secs_f32());
            }
            NormCeilingHalflife(v) => {
                self.processor_settings
                    .norm_ceiling_halflife
                    .set(v.as_secs_f32());
            }
        };
        emitter.emit_audio_state_change(sc);
    }

    /// Return the current value of the audio envelope.
    pub fn envelope(&self) -> UnipolarFloat {
        self.envelope_value
    }

    /// Return the name of the audio device.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Return whether monitoring is enabled.
    pub fn monitor(&self) -> bool {
        self.monitor
    }
}

#[derive(Debug, Clone)]
pub enum StateChange {
    Monitor(bool),
    EnvelopeValue(UnipolarFloat),
    EnvelopeAttack(Duration),
    EnvelopeRelease(Duration),
    OutputSmoothing(Duration),
    ActiveRole(Role),
    NormFloorHalflife(Duration),
    /// Ceiling half-life, in seconds of music at the reference motion rate.
    NormCeilingHalflife(Duration),
}

#[derive(Debug, Clone)]
pub enum ControlMessage {
    Set(StateChange),
    ToggleMonitor,
    ResetParameters,
}

pub trait EmitStateChange {
    fn emit_audio_state_change(&mut self, sc: StateChange);
}

/// Prompt the user to configure an audio input device.
pub fn prompt_audio() -> Result<Option<String>> {
    if !prompt_bool("Use audio input?")? {
        return Ok(None);
    }
    let input_devices = AudioInput::devices()?;
    if input_devices.is_empty() {
        bail!("No audio input devices found.");
    }
    println!("Available devices:");
    for (i, port) in input_devices.iter().enumerate() {
        println!("{i}: {port}");
    }
    prompt_indexed_value("Input audio device:", &input_devices).map(Some)
}

#[cfg(test)]
mod test {
    use super::*;

    /// Clipping is reported as soon as it starts, again every interval for as
    /// long as it continues, and not at all once it stops.
    #[test]
    fn clipping_is_reported_when_it_starts_and_while_it_lasts() {
        let mut reporter = ClipReporter::default();
        let step = Duration::from_millis(100);
        let mut clips = 0;
        let mut reported_at = Vec::new();
        // 25 s of continuous clipping: one new run every step.
        for i in 0..250 {
            clips += 1;
            if reporter.update(clips, step).is_some() {
                reported_at.push(i);
            }
        }
        assert_eq!(reported_at, [0, 100, 200], "reports while clipping");
        // 30 s of clean input.
        for _ in 0..300 {
            assert_eq!(
                reporter.update(clips, step),
                None,
                "quiet input is not reported"
            );
        }
        assert_eq!(
            reporter.update(clips + 3, step),
            Some(3),
            "a new bout is reported with its run count"
        );
    }

    /// The trim is reported to the nearest half dB, and only when it crosses
    /// to a new step.
    #[test]
    fn trim_is_reported_when_it_moves_a_step() {
        let mut input = AudioInput::offline();
        let set_db = |input: &AudioInput, db: f32| {
            input
                .processor_settings
                .trim_gain
                .set(10f32.powf(db / 20.0));
        };
        assert!(!input.trim_moved(), "unity trim is where it starts");
        set_db(&input, 3.1);
        assert!(input.trim_moved());
        assert_eq!(input.snapshot().trim_db, 3.0);
        set_db(&input, 3.2);
        assert!(!input.trim_moved(), "still nearest 3.0 dB");
        set_db(&input, 3.4);
        assert!(input.trim_moved());
        assert_eq!(input.snapshot().trim_db, 3.5);
        set_db(&input, -9.8);
        assert!(input.trim_moved());
        assert_eq!(input.snapshot().trim_db, -10.0);
    }
}
