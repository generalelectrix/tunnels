use crate::clock::ControllableClock;
use crate::clock_bank::{ClockIdxExt, ClockStore};
use crate::master_ui::EmitStateChange as EmitShowStateChange;
use crate::waveforms::WaveformArgs;
use crate::{clock::Clock, clock_bank::ClockBank};
use crate::{clock_bank::ClockIdx, waveforms};
use log::error;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Sawtooth,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
pub enum Target {
    Rotation,
    Thickness,
    Size,
    AspectRatio,
    Color,
    ColorSpread,
    ColorPeriodicity,
    ColorSaturation,
    MarqueeRotation,
    PositionX,
    PositionY,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Animation {
    pub waveform: Waveform,
    pulse: bool,
    standing: bool,
    invert: bool,
    n_periods: i32,
    pub target: Target,
    size: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    smoothing: UnipolarFloat,
    internal_clock: Clock,
    clock_source: Option<ClockIdx>,
    use_audio_size: bool,
}

impl Default for Animation {
    fn default() -> Self {
        Self::new()
    }
}

impl Animation {
    pub fn new() -> Self {
        Self {
            waveform: Waveform::Sine,
            pulse: false,
            standing: false,
            invert: false,
            n_periods: 0,
            target: Target::Size,
            size: UnipolarFloat::ZERO,
            duty_cycle: UnipolarFloat::ONE,
            smoothing: UnipolarFloat::new(0.25),
            internal_clock: Clock::new(),
            clock_source: None,
            use_audio_size: false,
        }
    }

    /// Return true if this animation has nonzero size.
    fn active(&self) -> bool {
        self.size > 0.0
    }

    fn phase(&self, external_clocks: &impl ClockStore) -> Phase {
        match self.clock_source {
            None => self.internal_clock.phase(),
            Some(id) => external_clocks.phase(id),
        }
    }

    /// Return the clock's current rate, scaled into a bipolar float.
    fn clock_speed(&self) -> BipolarFloat {
        return BipolarFloat::new(self.internal_clock.rate / ControllableClock::RATE_SCALE);
    }

    /// Set the clock's current rate, scaling by our scale factor.
    fn set_clock_speed(&mut self, speed: BipolarFloat) {
        self.internal_clock.rate = speed.val() * ControllableClock::RATE_SCALE;
    }

    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        if self.active() {
            self.internal_clock.update_state(delta_t, audio_envelope);
        }
    }

    pub fn get_value(
        &self,
        spatial_phase_offset: Phase,
        external_clocks: &impl ClockStore,
        audio_envelope: UnipolarFloat,
    ) -> f64 {
        if !self.active() {
            return 0.;
        }
        let waveform_func = match self.waveform {
            Waveform::Sine => waveforms::sine,
            Waveform::Square => waveforms::square,
            Waveform::Sawtooth => waveforms::sawtooth,
            Waveform::Triangle => waveforms::triangle,
        };
        let mut result = self.size.val()
            * waveform_func(&WaveformArgs {
                phase_spatial: spatial_phase_offset * (self.n_periods as f64),
                phase_temporal: self.phase(external_clocks),
                smoothing: self.smoothing,
                duty_cycle: self.duty_cycle,
                pulse: self.pulse,
                standing: self.standing,
            });

        // scale this animation by submaster level if using external clock
        let mut use_audio_size = self.use_audio_size;
        if let Some(id) = self.clock_source {
            result *= external_clocks.submaster_level(id).val();
            use_audio_size = use_audio_size || external_clocks.use_audio_size(id);
        }
        // scale this animation by audio envelope if set
        if use_audio_size {
            result *= audio_envelope.val();
        }
        if self.invert {
            -1.0 * result
        } else {
            result
        }
    }

    /// Emit the current value of all controllable animator state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_animation_state_change(Waveform(self.waveform));
        emitter.emit_animation_state_change(Pulse(self.pulse));
        emitter.emit_animation_state_change(Standing(self.standing));
        emitter.emit_animation_state_change(Invert(self.invert));
        emitter.emit_animation_state_change(NPeriods(self.n_periods));
        emitter.emit_animation_state_change(Target(self.target));
        emitter.emit_animation_state_change(Speed(self.clock_speed()));
        emitter.emit_animation_state_change(Size(self.size));
        emitter.emit_animation_state_change(DutyCycle(self.duty_cycle));
        emitter.emit_animation_state_change(Smoothing(self.smoothing));
        emitter.emit_animation_state_change(ClockSource(self.clock_source));
        emitter.emit_animation_state_change(UseAudioSize(self.use_audio_size));
        emitter.emit_animation_state_change(UseAudioSpeed(self.internal_clock.use_audio));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
            SetClockSource(source) => {
                let source: Option<ClockIdx> = match source {
                    Some(s) => match s.try_into() {
                        Ok(s) => Some(s),
                        Err(e) => {
                            error!("could not process animation control message: {e}");
                            return;
                        }
                    },
                    None => None,
                };
                self.handle_state_change(StateChange::ClockSource(source), emitter);
            }
            TogglePulse => {
                self.pulse = !self.pulse;
                emitter.emit_animation_state_change(StateChange::Pulse(self.pulse));
            }
            ToggleStanding => {
                self.standing = !self.standing;
                emitter.emit_animation_state_change(StateChange::Standing(self.standing));
            }
            ToggleInvert => {
                self.invert = !self.invert;
                emitter.emit_animation_state_change(StateChange::Invert(self.invert));
            }
            ToggleUseAudioSize => {
                self.use_audio_size = !self.use_audio_size;
                emitter.emit_animation_state_change(StateChange::UseAudioSize(self.use_audio_size));
            }
            ToggleUseAudioSpeed => {
                self.internal_clock.use_audio = !self.internal_clock.use_audio;
                emitter.emit_animation_state_change(StateChange::UseAudioSpeed(
                    self.internal_clock.use_audio,
                ));
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            Waveform(v) => self.waveform = v,
            Pulse(v) => self.pulse = v,
            Standing(v) => self.standing = v,
            Invert(v) => self.invert = v,
            NPeriods(v) => self.n_periods = v,
            Target(v) => self.target = v,
            Speed(v) => self.set_clock_speed(v),
            Size(v) => self.size = v,
            DutyCycle(v) => self.duty_cycle = v,
            Smoothing(v) => self.smoothing = v,
            ClockSource(v) => self.clock_source = v,
            UseAudioSize(v) => self.use_audio_size = v,
            UseAudioSpeed(v) => self.internal_clock.use_audio = v,
        };
        emitter.emit_animation_state_change(sc);
    }
}

#[derive(Debug)]
pub enum StateChange {
    Waveform(Waveform),
    Pulse(bool),
    Standing(bool),
    Invert(bool),
    NPeriods(i32),
    Target(Target),
    Speed(BipolarFloat),
    Size(UnipolarFloat),
    DutyCycle(UnipolarFloat),
    Smoothing(UnipolarFloat),
    ClockSource(Option<ClockIdx>),
    UseAudioSize(bool),
    UseAudioSpeed(bool),
}

pub enum ControlMessage {
    Set(StateChange),
    /// Since clock IDs need to be validated, this path handles the fallible case.
    /// FIXME: it would be nicer to validate this at control message creation time,
    /// but at the moment control message creator functions are infallible and
    /// that's more refactoring than I want to deal with right now.
    SetClockSource(Option<ClockIdxExt>),
    TogglePulse,
    ToggleStanding,
    ToggleInvert,
    ToggleUseAudioSize,
    ToggleUseAudioSpeed,
}

pub trait EmitStateChange {
    fn emit_animation_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_animation_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Animation(sc))
    }
}
