pub mod hilbert;
pub mod log_scale;
pub mod processor;
pub mod reconnect;
pub mod ring_buffer;
pub mod wavelet;

use anyhow::{Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{info, warn};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;
use tunnels_lib::prompt::{prompt_bool, prompt_indexed_value};
use tunnels_lib::transient_indicator::TransientIndicator;

pub use self::processor::UpdateRate;
use self::processor::{NUM_OUTPUT_BANDS, ProcessorSettings, TrackingMode};
use self::reconnect::ReconnectingInput;
pub use self::ring_buffer::EnvelopeStream;

/// Device name used when no audio device is connected.
pub const OFFLINE_DEVICE_NAME: &str = "Offline";

/// Envelope data streams from the audio thread, bundled with the callback rate.
pub struct EnvelopeStreams {
    pub streams: [EnvelopeStream; NUM_OUTPUT_BANDS],
    pub update_rate: UpdateRate,
}

/// A flat, read-only view of the audio input's current parameter state.
#[derive(Debug, Clone)]
pub struct AudioSnapshot {
    pub device_name: String,
    pub filter_cutoff_hz: f32,
    pub envelope_attack: Duration,
    pub envelope_release: Duration,
    pub output_smoothing: Duration,
    pub gain_linear: f64,
    pub auto_trim_enabled: bool,
    pub active_band: u32,
    pub norm_floor_halflife: Duration,
    pub norm_ceiling_halflife: Duration,
    pub norm_floor_mode: TrackingMode,
    pub norm_ceiling_mode: TrackingMode,
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self {
            device_name: OFFLINE_DEVICE_NAME.to_string(),
            filter_cutoff_hz: 200.0,
            envelope_attack: Duration::from_millis(10),
            envelope_release: Duration::from_millis(50),
            output_smoothing: Duration::from_millis(8),
            gain_linear: 1.0,
            auto_trim_enabled: true,
            active_band: 0,
            norm_floor_halflife: Duration::from_secs(10),
            norm_ceiling_halflife: Duration::from_secs(5),
            norm_floor_mode: TrackingMode::Average,
            norm_ceiling_mode: TrackingMode::Limit,
        }
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
    /// Transient envelope clip indicator.
    clip_indicator: TransientIndicator,
    /// Name of the audio device, or "Offline" if no device is connected.
    device_name: String,
}

impl AudioInput {
    const CLIP_INDICATOR_DURATION: Duration = Duration::from_millis(100);
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
            clip_indicator: TransientIndicator::new(Self::CLIP_INDICATOR_DURATION),
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
            clip_indicator: TransientIndicator::new(Self::CLIP_INDICATOR_DURATION),
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
        let ps = &self.processor_settings;
        AudioSnapshot {
            device_name: self.device_name.clone(),
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
        }
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
            let clipping = envelope > 1.0;
            if let Some(clip_state) = self.clip_indicator.update_state(delta_t, clipping) {
                emitter.emit_audio_state_change(StateChange::IsClipping(clip_state));
            }
        }
    }

    /// Emit the current value of all controllable state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_audio_state_change(EnvelopeValue(self.envelope_value));
        emitter.emit_audio_state_change(Monitor(self.monitor));
        emitter.emit_audio_state_change(FilterCutoff(self.processor_settings.filter_cutoff.get()));
        emitter.emit_audio_state_change(EnvelopeAttack(Duration::from_secs_f32(
            self.processor_settings.envelope_attack.get(),
        )));
        emitter.emit_audio_state_change(EnvelopeRelease(Duration::from_secs_f32(
            self.processor_settings.envelope_release.get(),
        )));
        emitter.emit_audio_state_change(OutputSmoothing(Duration::from_secs_f32(
            self.processor_settings.output_smoothing.get(),
        )));
        emitter.emit_audio_state_change(AutoTrimEnabled(
            self.processor_settings
                .auto_trim_enabled
                .load(Ordering::Relaxed),
        ));
        emitter.emit_audio_state_change(InputGain(self.processor_settings.gain.get() as f64));
        emitter.emit_audio_state_change(IsClipping(self.clip_indicator.state()));
        emitter.emit_audio_state_change(ActiveBand(
            self.processor_settings.active_band.load(Ordering::Relaxed),
        ));
        emitter.emit_audio_state_change(NormFloorHalflife(Duration::from_secs_f32(
            self.processor_settings.norm_floor_halflife.get(),
        )));
        emitter.emit_audio_state_change(NormCeilingHalflife(Duration::from_secs_f32(
            self.processor_settings.norm_ceiling_halflife.get(),
        )));
        emitter.emit_audio_state_change(NormFloorMode(
            self.processor_settings
                .norm_floor_mode
                .load(Ordering::Relaxed),
        ));
        emitter.emit_audio_state_change(NormCeilingMode(
            self.processor_settings
                .norm_ceiling_mode
                .load(Ordering::Relaxed),
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
                    emitter.emit_audio_state_change(StateChange::IsClipping(false));
                    self.clip_indicator.reset();
                }
            }
            ResetParameters => {
                self.processor_settings.reset_defaults();
                self.clip_indicator.reset();
                self.emit_state(emitter);
            }
            Set(sc) => self.handle_state_change(sc, emitter),
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            EnvelopeValue(_) | IsClipping(_) => return, // output only
            Monitor(v) => self.monitor = v,
            FilterCutoff(v) => {
                if v <= 0. {
                    warn!("Invalid filter cutoff frequency {v} (<= 0).");
                    return;
                }
                self.processor_settings.filter_cutoff.set(v);
            }
            EnvelopeAttack(v) => self.processor_settings.envelope_attack.set(v.as_secs_f32()),
            EnvelopeRelease(v) => self
                .processor_settings
                .envelope_release
                .set(v.as_secs_f32()),
            OutputSmoothing(v) => self
                .processor_settings
                .output_smoothing
                .set(v.as_secs_f32()),
            AutoTrimEnabled(v) => {
                self.processor_settings
                    .auto_trim_enabled
                    .store(v, Ordering::Relaxed);
            }
            InputGain(v) => {
                if v < 0. {
                    warn!("Invalid input gain {v} (< 0).");
                    return;
                }
                self.processor_settings.gain.set(v as f32);
            }
            ActiveBand(v) => {
                let clamped = v.min((NUM_OUTPUT_BANDS - 1) as u32);
                self.processor_settings
                    .active_band
                    .store(clamped, Ordering::Relaxed);
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
            NormFloorMode(v) => {
                self.processor_settings
                    .norm_floor_mode
                    .store(v, Ordering::Relaxed);
            }
            NormCeilingMode(v) => {
                self.processor_settings
                    .norm_ceiling_mode
                    .store(v, Ordering::Relaxed);
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

    /// Return the clip indicator state.
    pub fn is_clipping(&self) -> bool {
        self.clip_indicator.state()
    }
}

#[derive(Debug, Clone)]
pub enum StateChange {
    Monitor(bool),
    EnvelopeValue(UnipolarFloat),
    FilterCutoff(f32),
    EnvelopeAttack(Duration),
    EnvelopeRelease(Duration),
    OutputSmoothing(Duration),
    AutoTrimEnabled(bool),
    InputGain(f64),
    IsClipping(bool),
    ActiveBand(u32),
    NormFloorHalflife(Duration),
    NormCeilingHalflife(Duration),
    NormFloorMode(processor::TrackingMode),
    NormCeilingMode(processor::TrackingMode),
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
