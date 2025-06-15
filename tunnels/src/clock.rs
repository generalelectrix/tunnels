use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};

use crate::transient_indicator::TransientIndicator;

/// The number of times a clock has ticked.
/// Signed to support negative rates.
pub type Ticks = i64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clock {
    /// The current phase of this clock.
    phase: Phase,
    /// The total number of ticks this clock has made.
    ticks: Ticks,
    /// in unit angle per second
    pub rate: f64,
    /// did the clock tick on its most recent update?
    ticked: bool,
    /// is this clock running in "one-shot" mode?
    /// the clock runs for one cycle when triggered then waits for another
    /// trigger event
    one_shot: bool,
    /// should this clock run?
    run: bool,
    /// should this clock reset and tick on the next state update action?
    reset_on_update: bool,
    /// Should this clock scale its rate during update by the audio envelope?
    pub use_audio: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Self {
            phase: Phase::ZERO,
            ticks: 0,
            rate: 0.0,
            ticked: true,
            one_shot: false,
            reset_on_update: false,
            run: true,
            use_audio: false,
        }
    }

    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        if self.reset_on_update {
            self.ticked = true;
            self.ticks = 0;
            // Reset phase to zero or one, depending on sign of rate.
            self.phase = if self.rate >= 0.0 {
                Phase::ZERO
            } else {
                Phase::ONE
            };
            self.reset_on_update = false;
            self.run = true;
            return;
        }

        if !self.run {
            return;
        }

        let rate_modulation = if self.use_audio {
            audio_envelope
        } else {
            UnipolarFloat::ONE
        };

        let new_angle =
            self.phase.val() + (self.rate * rate_modulation.val() * delta_t.as_secs_f64());

        // if we're running in one-shot mode, clamp the angle at 1.0
        if self.one_shot && new_angle >= 1.0 {
            self.phase = Phase::ONE;
            self.ticked = false;
            self.run = false;
        } else if self.one_shot && new_angle < 0.0 {
            self.phase = Phase::ZERO;
            self.ticked = false;
            self.run = false;
        } else {
            // if the phase just escaped our range, we ticked this frame
            self.ticked = !(0.0..1.0).contains(&new_angle);
            if self.ticked {
                self.ticks = self.ticks.wrapping_add(new_angle.div_euclid(1.0) as i64);
            }
            self.phase = Phase::new(new_angle);
        }
    }

    fn set_one_shot(&mut self, one_shot: bool) {
        self.one_shot = one_shot;
        if !one_shot {
            self.run = true;
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn ticks(&self) -> Ticks {
        self.ticks
    }
}

/// A static snapshot of externally-visible ControllableClock state.
#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct StaticClock {
    pub phase: Phase,
    pub ticks: Ticks,
    pub submaster_level: UnipolarFloat,
    pub use_audio_size: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A clock with a complete set of controls.
pub struct ControllableClock {
    clock: Clock,
    sync: TapSync,
    tick_indicator: TransientIndicator,
    /// If true, reset the clock's phase to zero on every tap.
    retrigger: bool,
    /// submaster level for this clock
    submaster_level: UnipolarFloat,
    /// If true, modulate the submaster level using audio envelope.
    use_audio_size: bool,
}

impl Default for ControllableClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllableClock {
    /// radial units/s, permitting a max internal clock rate of 1.5 Hz
    /// the negative sign is here so that turning the animation speed knob
    /// clockwise makes the animation appear to run around the beam in the same
    /// direction
    pub const RATE_SCALE: f64 = -1.5;

    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            sync: TapSync::new(),
            tick_indicator: TransientIndicator::new(Duration::from_millis(100)),
            retrigger: false,
            submaster_level: UnipolarFloat::ONE,
            use_audio_size: false,
        }
    }

    /// Return the current phase of this clock.
    pub fn phase(&self) -> Phase {
        self.clock.phase()
    }

    /// Return the number of ticks this clock has ticked.
    pub fn ticks(&self) -> Ticks {
        self.clock.ticks()
    }

    /// Return the current submaster level.
    pub fn submaster_level(&self) -> UnipolarFloat {
        self.submaster_level
    }

    /// Return true if we should use audio envelope to scale submaster level.
    /// This is returned independently, rather than applied to the submaster
    /// level directly, to allow clients of this submaster to avoid double-
    /// modulating with audio envelope.
    pub fn use_audio_size(&self) -> bool {
        self.use_audio_size
    }

    /// Get all clock state bundled into a struct.
    pub fn as_static(&self) -> StaticClock {
        StaticClock {
            phase: self.phase(),
            ticks: self.ticks(),
            submaster_level: self.submaster_level(),
            use_audio_size: self.use_audio_size(),
        }
    }

    /// Update the state of this clock.
    /// The clock may need to emit state update messages.
    pub fn update_state<E: EmitStateChange>(
        &mut self,
        delta_t: Duration,
        audio_envelope: UnipolarFloat,
        emitter: &mut E,
    ) {
        self.clock.update_state(delta_t, audio_envelope);
        if let Some(tick_state) = self.tick_indicator.update_state(delta_t, self.clock.ticked) {
            emitter.emit_clock_state_change(StateChange::Ticked(tick_state));
        }
    }

    /// Emit the current value of all controllable state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_clock_state_change(Retrigger(self.retrigger));
        emitter.emit_clock_state_change(OneShot(self.clock.one_shot));
        emitter.emit_clock_state_change(SubmasterLevel(self.submaster_level));
        emitter.emit_clock_state_change(Ticked(self.tick_indicator.state()));
        emitter.emit_clock_state_change(UseAudioSpeed(self.clock.use_audio));
        emitter.emit_clock_state_change(UseAudioSize(self.use_audio_size));
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
                } else if let Some(rate) = self.sync.tap() {
                    self.clock.rate = rate;
                    emitter.emit_clock_state_change(StateChange::Rate(BipolarFloat::new(
                        self.clock.rate / ControllableClock::RATE_SCALE,
                    )));
                }
            }
            ToggleOneShot => {
                self.handle_state_change(StateChange::OneShot(!self.clock.one_shot), emitter);
            }
            ToggleRetrigger => {
                self.handle_state_change(StateChange::Retrigger(!self.retrigger), emitter);
            }
            ToggleUseAudioSize => {
                self.handle_state_change(StateChange::UseAudioSize(!self.use_audio_size), emitter);
            }
            ToggleUseAudioSpeed => {
                self.handle_state_change(
                    StateChange::UseAudioSpeed(!self.clock.use_audio),
                    emitter,
                );
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            Rate(v) => self.clock.rate = v.val() * ControllableClock::RATE_SCALE,
            Retrigger(v) => self.retrigger = v,
            OneShot(v) => self.clock.set_one_shot(v),
            SubmasterLevel(v) => self.submaster_level = v,
            UseAudioSpeed(v) => self.clock.use_audio = v,
            UseAudioSize(v) => self.use_audio_size = v,
            Ticked(_) => (),
        };
        emitter.emit_clock_state_change(sc);
    }
}

#[derive(Debug, Clone)]
pub enum StateChange {
    Rate(BipolarFloat),
    Retrigger(bool),
    OneShot(bool),
    SubmasterLevel(UnipolarFloat),
    UseAudioSize(bool),
    UseAudioSpeed(bool),
    /// Outgoing only, no effect as control.
    Ticked(bool),
}

#[derive(Debug, Clone)]
pub enum ControlMessage {
    Set(StateChange),
    Tap,
    ToggleOneShot,
    ToggleRetrigger,
    ToggleUseAudioSize,
    ToggleUseAudioSpeed,
}

pub trait EmitStateChange {
    fn emit_clock_state_change(&mut self, sc: StateChange);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Estimate rate from a series of taps.
struct TapSync {
    #[serde(skip)]
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
        if let (Some(first), Some(last)) = (self.taps.first(), self.taps.last()) {
            let period = (*last - *first) / (self.taps.len() as u32 - 1);
            self.period = Some(period);
            self.rate = Some(1.0 / period.as_secs_f64());
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
                let abs_difference = if period > dt {
                    period - dt
                } else {
                    dt - period
                };
                let fractional_difference = abs_difference.as_secs_f64() / period.as_secs_f64();

                if fractional_difference > Self::RESET_THRESHOLD {
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
