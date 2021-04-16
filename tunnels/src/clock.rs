use crate::master_ui::EmitStateChange as EmitShowStateChange;
use crate::numbers::UnipolarFloat;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// how many globally-available clocks?
pub const N_CLOCKS: usize = 4;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ClockIdx(pub usize);

/// Maintain a indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockBank([Clock; N_CLOCKS]);

impl ClockBank {
    pub fn new() -> Self {
        Self(Default::default())
    }

    pub fn curr_angle(&self, index: ClockIdx) -> UnipolarFloat {
        self.0[index.0].curr_angle()
    }

    pub fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat {
        self.0[index.0].submaster_level
    }

    pub fn update_state(&mut self, delta_t: Duration) {
        for clock in self.0.iter_mut() {
            clock.update_state(delta_t);
        }
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
    /// should this clock reset and tick on the next state update action?
    reset_on_update: bool,
    /// submaster level for this clock
    pub submaster_level: UnipolarFloat,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
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
}

/// A clock with a complete set of controls.
pub struct ControllableClock {
    clock: Clock,
    sync: TapSync,
    tick_age: Option<Duration>,
    /// If true, reset the clock's phase to zero on every tap.
    retrigger: bool,
}

impl ControllableClock {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            sync: TapSync::new(),
            tick_age: None,
            retrigger: false,
        }
    }

    const TICK_DISPLAY_DURATION: Duration = Duration::from_millis(250);

    /// Update the state of this clock.
    /// The clock may need to emit state update messages.
    pub fn update_state<E: EmitStateChange>(&mut self, delta_t: Duration, emitter: &mut E) {
        self.clock.update_state(delta_t);
        if self.clock.ticked {
            emitter.emit_clock_state_change(StateChange::Ticked(true));
            self.tick_age = Some(Duration::new(0, 0));
        } else if let Some(tick_age) = self.tick_age {
            let new_tick_age = tick_age + delta_t;
            if new_tick_age > Self::TICK_DISPLAY_DURATION {
                self.tick_age = None;
                emitter.emit_clock_state_change(StateChange::Ticked(false));
            } else {
                self.tick_age = Some(new_tick_age);
            }
        }
    }

    fn tick_indicator_state(&self) -> bool {
        if let Some(age) = self.tick_age {
            age < Self::TICK_DISPLAY_DURATION
        } else {
            false
        }
    }

    /// Emit the current value of all controllable state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_clock_state_change(Retrigger(self.retrigger));
        emitter.emit_clock_state_change(OneShot(self.clock.one_shot));
        emitter.emit_clock_state_change(SubmasterLevel(self.clock.submaster_level));
        emitter.emit_clock_state_change(Ticked(self.tick_indicator_state()));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
            Tap => {
                if self.retrigger {
                    self.clock.reset_on_update = true;
                } else {
                    if let Some(rate) = self.sync.tap() {
                        self.clock.rate = rate;
                    }
                }
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            Retrigger(v) => self.retrigger = v,
            OneShot(v) => self.clock.one_shot = v,
            SubmasterLevel(v) => self.clock.submaster_level = v,
            Ticked(_) => (),
        };
        emitter.emit_clock_state_change(sc);
    }
}

pub enum StateChange {
    Retrigger(bool),
    OneShot(bool),
    SubmasterLevel(UnipolarFloat),
    /// Outgoing only, no effect as control.
    Ticked(bool),
}

pub enum ControlMessage {
    Set(StateChange),
    Tap,
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

    /// Process a tap event.  Return our new rate estimate if we have one.
    pub fn tap(&mut self) -> Option<f64> {
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
        self.rate
    }
}
