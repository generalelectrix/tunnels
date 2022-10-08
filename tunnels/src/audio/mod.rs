use crate::master_ui::EmitStateChange as EmitShowStateChange;
use crate::transient_indicator::TransientIndicator;
use cpal::traits::{DeviceTrait, HostTrait};
use log::{info, warn};
use std::error::Error;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;

use self::processor::ProcessorSettings;
use self::reconnect::ReconnectingInput;

mod processor;
mod reconnect;

pub struct AudioInput {
    _input: Option<ReconnectingInput>,
    processor_settings: ProcessorSettings,
    /// Locally-stored value of the envelope.
    envelope_value: UnipolarFloat,
    /// Should we send monitor updates?
    monitor: bool,
    /// Envelope gain factor.
    gain: f64,
    /// Transient envelope clip indicator.
    clip_indicator: TransientIndicator,
}

impl AudioInput {
    const CLIP_INDICATOR_DURATION: Duration = Duration::from_millis(100);
    /// Get the names of all available input audio devices.
    pub fn devices() -> Result<Vec<String>, Box<dyn Error>> {
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
            gain: 1.0,
            clip_indicator: TransientIndicator::new(Self::CLIP_INDICATOR_DURATION),
        }
    }

    pub fn new(device_name: Option<String>) -> Result<Self, Box<dyn Error>> {
        let device_name = match device_name {
            None => {
                return Ok(Self::offline());
            }
            Some(d) => d,
        };

        info!("Using audio input device {}.", device_name);

        let processor_settings = ProcessorSettings::default();

        let input = ReconnectingInput::new(device_name, processor_settings.clone());

        Ok(Self {
            _input: Some(input),
            processor_settings,
            envelope_value: UnipolarFloat::ZERO,
            monitor: false,
            gain: 1.0,
            clip_indicator: TransientIndicator::new(Self::CLIP_INDICATOR_DURATION),
        })
    }

    /// Update the state of audio control.
    /// The audio control system may need to emit state update.
    pub fn update_state<E: EmitStateChange>(&mut self, delta_t: Duration, emitter: &mut E) {
        let raw_envelope = self.processor_settings.envelope.get() as f64;
        let scaled_envelope = raw_envelope * self.gain;
        let clipping = scaled_envelope > 1.0;
        self.envelope_value = UnipolarFloat::new(scaled_envelope);
        if self.monitor {
            emitter.emit_audio_state_change(StateChange::EnvelopeValue(self.envelope_value));
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
        emitter.emit_audio_state_change(Gain(self.gain));
        emitter.emit_audio_state_change(IsClipping(self.clip_indicator.state()));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
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
                self.gain = 1.0;
                self.clip_indicator.reset();
                self.emit_state(emitter);
            }
            Set(sc) => self.handle_state_change(sc, emitter),
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            EnvelopeValue(_) => (), // output only, input ignored
            Monitor(v) => self.monitor = v,
            FilterCutoff(v) => {
                if v <= 0. {
                    warn!("Invalid filter cutoff frequency {} (<= 0).", v);
                    return;
                }
                self.processor_settings.filter_cutoff.set(v);
            }
            EnvelopeAttack(v) => self.processor_settings.envelope_attack.set(v.as_secs_f32()),
            EnvelopeRelease(v) => self
                .processor_settings
                .envelope_release
                .set(v.as_secs_f32()),
            Gain(v) => {
                if v < 0. {
                    warn!("Invalid audio envelope gain {} (< 0).", v);
                    return;
                }
                info!("Gain: {}", v);
                self.gain = v;
            }
            IsClipping(_) => {
                return; // output only
            }
        };
        emitter.emit_audio_state_change(sc);
    }

    /// Return the current value of the audio envelope.
    pub fn envelope(&self) -> UnipolarFloat {
        self.envelope_value
    }
}

#[derive(Debug)]
pub enum StateChange {
    Monitor(bool),
    EnvelopeValue(UnipolarFloat),
    FilterCutoff(f32),
    EnvelopeAttack(Duration),
    EnvelopeRelease(Duration),
    Gain(f64),
    IsClipping(bool),
}

pub enum ControlMessage {
    Set(StateChange),
    ToggleMonitor,
    ResetParameters,
}

pub trait EmitStateChange {
    fn emit_audio_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_audio_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Audio(sc))
    }
}
