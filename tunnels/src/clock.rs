use crate::numbers::UnipolarFloat;
use crate::ui::EmitStateChange as EmitShowStateChange;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// how many globally-available clocks?
const N_CLOCKS: usize = 8;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ClockIdx(pub usize);

/// Maintain a indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockBank([Clock; N_CLOCKS]);

impl ClockBank {
    pub fn curr_angle(&self, index: ClockIdx) -> UnipolarFloat {
        self.0[index.0].curr_angle()
    }

    pub fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat {
        self.0[index.0].submaster_level
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clock {
    curr_angle: UnipolarFloat,
    /// in unit angle per second
    pub rate: f64,
    /// did the clock tick on its most recent update?
    ticked: bool,
    /// is this clock running in "one-shot" mode?
    /// the clock runs for one cycle when triggered then waits for another
    /// trigger event
    one_shot: bool,
    /// should this clock reset and tick on its next update?
    reset_on_update: bool,
    /// submaster level for this clock
    pub submaster_level: UnipolarFloat,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            curr_angle: UnipolarFloat(0.0),
            rate: 0.0,
            ticked: true,
            one_shot: false,
            reset_on_update: false,
            submaster_level: UnipolarFloat(1.0),
        }
    }

    pub fn update_state(&mut self, delta_t: Duration) {
        if self.reset_on_update {
            self.ticked = true;
            self.curr_angle = UnipolarFloat(0.0);
            self.reset_on_update = false;
        } else {
            // delta_t has units of us, need to divide by 1000000
            let new_angle = self.curr_angle.0 + (self.rate * delta_t.as_secs_f64());

            // if we're running in one-shot mode, clamp the angle at 1.0
            if self.one_shot && new_angle >= 1.0 {
                self.curr_angle = UnipolarFloat(1.0);
                self.ticked = false;
            } else {
                // if the phase just escaped our range, we ticked this frame
                self.ticked = new_angle >= 1.0 || new_angle < 0.0;
                self.curr_angle = UnipolarFloat(new_angle % 1.0);
            }
        }
    }

    pub fn curr_angle(&self) -> UnipolarFloat {
        self.curr_angle
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {};
        emitter.emit_clock_state_change(sc);
    }
}

/// A clock with a complete set of controls.
pub struct ControllableClock {
    clock: Clock,
    sync: TapSync,
    tick_age: Option<Duration>,
}

pub enum StateChange {}

pub enum ControlMessage {
    Set(StateChange),
}

pub trait EmitStateChange {
    fn emit_clock_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_clock_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Clock(sc))
    }
}

/// Estimate rate from a series of taps.
struct TapSync {
    taps: Vec<Instant>,
    rate: Option<f64>,
    period: Option<Duration>,
}

impl TapSync {
    /// Fractional threshold at which we'll discard the current tap buffer and
    /// start a new one.
    const RESET_THRESHOLD: f64 = 0.1;

    pub fn new() -> Self {
        Self {
            taps: Vec::new(),
            rate: None,
            period: None,
        }
    }

    fn reset_buffer(&mut self, tap: Instant) {
        self.taps.clear();
        self.taps.push(tap);
        self.rate = None;
        self.period = None;
    }

    fn add_tap(&mut self, tap: Instant) {
        self.taps.push(tap);
        if self.taps.len() < 2 {
            return;
        }
        // compute rate if we have at least two taps
        match (self.taps.first(), self.taps.last()) {
            (Some(first), Some(last)) => {
                let period = (*last - *first) / (self.taps.len() as u32 - 1);
                self.period = Some(period);
                self.rate = Some(1.0 / period.as_secs_f64());
            }
            _ => (),
        }
    }

    pub fn tap(&mut self) {
        let tap = Instant::now();
        // if the tap buffer isn't empty, determine elapsed time from the last
        // tap to this one
        match self.period {
            Some(period) => {
                let dt = tap - *self.taps.last().unwrap();

                // if this single estimate of tempo is within +-10% of current, use it
                // otherwise, empty the buffer and start over
                let fractional_difference = (period - dt).as_secs_f64() / period.as_secs_f64();

                if fractional_difference.abs() > Self::RESET_THRESHOLD {
                    // outlier, empty the buffer
                    self.reset_buffer(tap);
                } else {
                    // append to buffer and update
                    self.add_tap(tap);
                }
            }
            None => self.add_tap(tap),
        }
    }
}
