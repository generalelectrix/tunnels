use crate::{clock::Clock, clock_bank::ClockBank, numbers::BipolarFloat};
use crate::{clock::ControllableClock, numbers::UnipolarFloat};
use crate::{clock_bank::ClockIdx, waveforms};
use crate::{master_ui::EmitStateChange as EmitShowStateChange, numbers::Phase};
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
    Segments,
    Blacking,
    PositionX,
    PositionY,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Animation {
    pub waveform: Waveform,
    pulse: bool,
    invert: bool,
    n_periods: i32,
    pub target: Target,
    weight: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    smoothing: UnipolarFloat,
    internal_clock: Clock,
    clock_source: Option<ClockIdx>,
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
            invert: false,
            n_periods: 0,
            target: Target::Size,
            weight: UnipolarFloat::new(0.0),
            duty_cycle: UnipolarFloat::new(1.0),
            smoothing: UnipolarFloat::new(0.25),
            internal_clock: Clock::new(),
            clock_source: None,
        }
    }

    /// Return true if this animation has nonzero weight.
    fn active(&self) -> bool {
        self.weight > 0.0
    }

    fn phase(&self, external_clocks: &ClockBank) -> Phase {
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

    pub fn update_state(&mut self, delta_t: Duration) {
        if self.active() {
            self.internal_clock.update_state(delta_t);
        }
    }

    pub fn get_value(&self, phase_offset: Phase, external_clocks: &ClockBank) -> f64 {
        if !self.active() {
            return 0.;
        }

        let angle = self.phase(external_clocks) + phase_offset * (self.n_periods as f64);
        let waveform_func = match self.waveform {
            Waveform::Sine => waveforms::sine,
            Waveform::Square => waveforms::square,
            Waveform::Sawtooth => waveforms::sawtooth,
            Waveform::Triangle => waveforms::triangle,
        };
        let mut result =
            self.weight.val() * waveform_func(angle, self.smoothing, self.duty_cycle, self.pulse);

        // scale this animation by submaster level if using external clock
        if let Some(id) = self.clock_source {
            result *= external_clocks.submaster_level(id).val();
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
        emitter.emit_animation_state_change(Invert(self.invert));
        emitter.emit_animation_state_change(NPeriods(self.n_periods));
        emitter.emit_animation_state_change(Target(self.target));
        emitter.emit_animation_state_change(Speed(self.clock_speed()));
        emitter.emit_animation_state_change(Weight(self.weight));
        emitter.emit_animation_state_change(DutyCycle(self.duty_cycle));
        emitter.emit_animation_state_change(Smoothing(self.smoothing));
        emitter.emit_animation_state_change(ClockSource(self.clock_source));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
            TogglePulse => {
                self.pulse = !self.pulse;
                emitter.emit_animation_state_change(StateChange::Pulse(self.pulse));
            }
            ToggleInvert => {
                self.invert = !self.invert;
                emitter.emit_animation_state_change(StateChange::Invert(self.invert));
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            Waveform(v) => self.waveform = v,
            Pulse(v) => self.pulse = v,
            Invert(v) => self.invert = v,
            NPeriods(v) => self.n_periods = v,
            Target(v) => self.target = v,
            Speed(v) => self.set_clock_speed(v),
            Weight(v) => self.weight = v,
            DutyCycle(v) => self.duty_cycle = v,
            Smoothing(v) => self.smoothing = v,
            ClockSource(v) => self.clock_source = v,
        };
        emitter.emit_animation_state_change(sc);
    }
}

#[derive(Debug)]
pub enum StateChange {
    Waveform(Waveform),
    Pulse(bool),
    Invert(bool),
    NPeriods(i32),
    Target(Target),
    Speed(BipolarFloat),
    Weight(UnipolarFloat),
    DutyCycle(UnipolarFloat),
    Smoothing(UnipolarFloat),
    ClockSource(Option<ClockIdx>),
}

pub enum ControlMessage {
    Set(StateChange),
    TogglePulse,
    ToggleInvert,
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
