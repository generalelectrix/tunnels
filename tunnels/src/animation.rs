use crate::clock::{Clock, ClockBank, ClockIdx};
use crate::numbers::UnipolarFloat;
use crate::waveforms;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Copy, Clone, Serialize, Deserialize)]
pub enum Waveform {
    Sine,
    Triangle,
    Square,
    Sawtooth,
}

#[derive(Copy, Clone, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct Animation {
    waveform: Waveform,
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
            weight: UnipolarFloat(0.0),
            duty_cycle: UnipolarFloat(1.0),
            smoothing: UnipolarFloat(0.25),
            internal_clock: Clock::new(),
            clock_source: None,
        }
    }

    /// Return true if this animation has nonzero weight.
    fn active(&self) -> bool {
        self.weight.0 > 0.0
    }

    fn clock_time(&self, external_clocks: &ClockBank) -> UnipolarFloat {
        match self.clock_source {
            None => self.internal_clock.curr_angle(),
            Some(id) => external_clocks.curr_angle(id),
        }
    }

    pub fn update_state(&mut self, delta_t: Duration) {
        if self.active() {
            self.internal_clock.update_state(delta_t);
        }
    }

    pub fn get_value(&self, angle_offset: f64, external_clocks: &ClockBank) -> f64 {
        if !self.active() {
            return 0.;
        }

        let angle = angle_offset * (self.n_periods as f64) + self.clock_time(external_clocks).0;
        let waveform_func = match self.waveform {
            Waveform::Sine => waveforms::sine,
            Waveform::Square => waveforms::square,
            Waveform::Sawtooth => waveforms::sawtooth,
            Waveform::Triangle => waveforms::triangle,
        };
        let scaled_smoothing = UnipolarFloat(self.smoothing.0 * WAVE_SMOOTHING_SCALE);
        let mut result =
            self.weight.0 * waveform_func(angle, scaled_smoothing, self.duty_cycle, self.pulse);

        // scale this animation by submaster level if using external clock
        if let Some(id) = self.clock_source {
            result *= external_clocks.submaster_level(id).0;
        }
        if self.invert {
            -1.0 * result
        } else {
            result
        }
    }
}

const WAVE_SMOOTHING_SCALE: f64 = 0.25;
