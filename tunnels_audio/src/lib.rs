//! Audio input and envelope extraction: the role envelopes (see [`roles`]),
//! derived on the audio thread from a resonator bank.
//!
//! The chain's behaviour is pinned by `tests/envelope_golden.rs` (response
//! shapes), `tests/music_convergence.rs` (long-term stability) and
//! `tests/alignment.rs` (independence from the buffer grid).

pub mod bank;
pub mod denormals;
pub mod input_meter;
pub mod processor;
pub mod reconnect;
pub mod ring_buffer;
pub mod roles;
pub mod time;

use anyhow::{Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use log::info;
use std::sync::Weak;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;
use tunnels_lib::prompt::{prompt_bool, prompt_indexed_value};

pub use self::input_meter::InputMeter;
pub use self::processor::UpdateRate;
use self::processor::{ProcessorSettings, ProcessorSettingsInner};
use self::reconnect::ReconnectingInput;
pub use self::ring_buffer::EnvelopeStream;
use self::roles::{NUM_ROLES, Role};
use self::time::HalfLife;

/// Device name used when no audio device is connected.
pub const OFFLINE_DEVICE_NAME: &str = "Offline";

/// The live output of one open audio input, for display: its envelope
/// streams, one per role in [`Role::ALL`] order, the rate they update at,
/// and its input meter.
pub struct EnvelopeStreams {
    pub streams: [EnvelopeStream; NUM_ROLES],
    pub update_rate: UpdateRate,
    /// The input's trim and clip indicator, which upgrade only while the
    /// input is open.
    pub input_meter: Weak<InputMeter>,
}

/// A flat, read-only view of the audio input's current parameter state.
#[derive(Debug, Clone)]
pub struct AudioSnapshot {
    pub device_name: String,
    pub envelope_attack: Duration,
    pub envelope_release: Duration,
    pub output_smoothing: Duration,
    pub active_role: Role,
    pub norm_floor_halflife: HalfLife,
    pub norm_ceiling_halflife: HalfLife,
}

impl AudioSnapshot {
    /// The parameter state a settings handle currently holds.
    fn read(device_name: &str, ps: &ProcessorSettingsInner) -> Self {
        Self {
            device_name: device_name.to_string(),
            envelope_attack: ps.envelope_attack.get(),
            envelope_release: ps.envelope_release.get(),
            output_smoothing: ps.output_smoothing.get(),
            active_role: Role::from_index(ps.active_role.load(Ordering::Relaxed) as usize)
                .unwrap_or(ProcessorSettingsInner::DEFAULT_ROLE),
            norm_floor_halflife: ps.norm_floor_halflife.get(),
            norm_ceiling_halflife: ps.norm_ceiling_halflife.get(),
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
    /// Name of the audio device, or "Offline" if no device is connected.
    device_name: String,
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

    /// Update the state of audio control.
    pub fn update_state<E: EmitStateChange>(&mut self, delta_t: Duration, emitter: &mut E) {
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

    /// Emit the current value of all controllable state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_audio_state_change(EnvelopeValue(self.envelope_value));
        emitter.emit_audio_state_change(Monitor(self.monitor));
        emitter.emit_audio_state_change(EnvelopeAttack(
            self.processor_settings.envelope_attack.get(),
        ));
        emitter.emit_audio_state_change(EnvelopeRelease(
            self.processor_settings.envelope_release.get(),
        ));
        emitter.emit_audio_state_change(OutputSmoothing(
            self.processor_settings.output_smoothing.get(),
        ));
        emitter.emit_audio_state_change(ActiveRole(self.snapshot().active_role));
        emitter.emit_audio_state_change(NormFloorHalflife(
            self.processor_settings.norm_floor_halflife.get(),
        ));
        emitter.emit_audio_state_change(NormCeilingHalflife(
            self.processor_settings.norm_ceiling_halflife.get(),
        ));
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
            EnvelopeAttack(v) => self.processor_settings.envelope_attack.set(v),
            EnvelopeRelease(v) => self.processor_settings.envelope_release.set(v),
            OutputSmoothing(v) => self.processor_settings.output_smoothing.set(v),
            ActiveRole(role) => {
                self.processor_settings
                    .active_role
                    .store(role.index() as u32, Ordering::Relaxed);
            }
            NormFloorHalflife(v) => self.processor_settings.norm_floor_halflife.set(v),
            NormCeilingHalflife(v) => self.processor_settings.norm_ceiling_halflife.set(v),
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
    NormFloorHalflife(HalfLife),
    /// Ceiling half-life, as a span of music at the reference motion rate.
    NormCeilingHalflife(HalfLife),
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
