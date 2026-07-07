use std::time::Duration;

use crate::{
    clock::{
        ControlMessage as ClockControlMessage, ControllableClock,
        EmitStateChange as EmitClockStateChange, StateChange as ClockStateChange, StaticClock,
        Ticks,
    },
    master_ui::EmitStateChange as EmitShowStateChange,
};
use arrayvec::ArrayVec;
use log::{error, warn};
use serde::{Deserialize, Serialize};
use tunnels_lib::number::{Phase, UnipolarFloat};

/// Read-only interface to the state of a collection of clocks.
pub trait ClockStore {
    /// Return the number of clocks in the bank.
    fn len(&self) -> usize;

    /// Return true if the bank contains no clocks.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the current phase of this clock.
    fn phase(&self, index: ClockIdx) -> Phase;

    /// Returnt the absolute number of ticks.
    fn ticks(&self, index: ClockIdx) -> Ticks;

    /// Return the current submaster level of this clock.
    fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat;

    /// Return true if we should use audio envelope to scale submaster level.
    /// This is returned independently, rather than applied to the submaster
    /// level directly, to allow clients of the submaster to avoid double-
    /// modulating with audio envelope.
    fn use_audio_size(&self, index: ClockIdx) -> bool;
}

/// The maximum number of clocks a bank can hold. The number in use is a runtime
/// value that can vary up to this bound.
pub const MAX_CLOCKS: usize = 12;

/// The number of clocks in a bank when no count is otherwise specified.
pub const DEFAULT_N_CLOCKS: usize = 4;

/// The number of clocks contributed by one clock wing.
pub const CLOCKS_PER_WING: usize = 4;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
/// Index of a clock in a bank.
///
/// Not a proof of validity: the clock count is dynamic, so a read for an
/// out-of-range index yields a neutral default rather than a value.
pub struct ClockIdx(pub usize);

/// Maintain an indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockBank(ArrayVec<ControllableClock, MAX_CLOCKS>);

impl Default for ClockBank {
    fn default() -> Self {
        Self::new(DEFAULT_N_CLOCKS)
    }
}

impl ClockStore for ClockBank {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn phase(&self, index: ClockIdx) -> Phase {
        self.get(index).map(|c| c.phase()).unwrap_or(Phase::ZERO)
    }

    fn ticks(&self, index: ClockIdx) -> Ticks {
        self.get(index).map(|c| c.ticks()).unwrap_or(0)
    }

    fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat {
        self.get(index)
            .map(|c| c.submaster_level())
            .unwrap_or(UnipolarFloat::ZERO)
    }

    fn use_audio_size(&self, index: ClockIdx) -> bool {
        self.get(index).map(|c| c.use_audio_size()).unwrap_or(false)
    }
}

impl ClockBank {
    /// Create a bank with `n` clocks, clamped to [`MAX_CLOCKS`].
    pub fn new(n: usize) -> Self {
        let n = n.min(MAX_CLOCKS);
        Self((0..n).map(|_| ControllableClock::default()).collect())
    }

    /// Grow or shrink the bank to hold `n` clocks, clamped to [`MAX_CLOCKS`].
    /// New clocks are added in their default state; removed clocks are dropped.
    pub fn set_clock_count(&mut self, n: usize) {
        let clamped = n.min(MAX_CLOCKS);
        if clamped != n {
            warn!("requested {n} clocks exceeds the maximum of {MAX_CLOCKS}; clamping.");
        }
        while self.0.len() < clamped {
            self.0.push(ControllableClock::default());
        }
        self.0.truncate(clamped);
    }

    pub fn update_state<E: EmitStateChange>(
        &mut self,
        delta_t: Duration,
        audio_envelope: UnipolarFloat,
        emitter: &mut E,
    ) {
        for (i, clock) in self.0.iter_mut().enumerate() {
            clock.update_state(
                delta_t,
                audio_envelope,
                &mut ChannelEmitter {
                    channel: ClockIdx(i),
                    emitter,
                },
            );
        }
    }

    /// Return the clock at `index`, or `None` if the index is out of range.
    pub fn get(&self, index: ClockIdx) -> Option<&ControllableClock> {
        self.0.get(index.0)
    }

    /// Return a static snapshot of the state of this clock bank.
    pub fn as_static(&self) -> ArrayVec<StaticClock, MAX_CLOCKS> {
        self.0.iter().map(|c| c.as_static()).collect()
    }

    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        for (i, clock) in self.0.iter().enumerate() {
            clock.emit_state(&mut ChannelEmitter {
                channel: ClockIdx(i),
                emitter,
            });
        }
    }

    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        let channel = msg.channel;
        let Some(clock) = self.0.get_mut(channel.0) else {
            error!(
                "could not process clock control message {msg:?}: channel {} is out of range for {} clocks",
                channel.0,
                self.0.len()
            );
            return;
        };
        clock.control(msg.msg, &mut ChannelEmitter { channel, emitter })
    }
}

/// Adds the clock channel into outgoing clock messages.
struct ChannelEmitter<'e, E: EmitStateChange> {
    channel: ClockIdx,
    emitter: &'e mut E,
}

impl<'e, E: EmitStateChange> EmitClockStateChange for ChannelEmitter<'e, E> {
    fn emit_clock_state_change(&mut self, sc: ClockStateChange) {
        self.emitter.emit_clock_bank_state_change(StateChange {
            channel: self.channel,
            change: sc,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ControlMessage {
    pub channel: ClockIdx,
    pub msg: ClockControlMessage,
}

#[derive(Debug, Clone)]
pub struct StateChange {
    pub channel: ClockIdx,
    pub change: ClockStateChange,
}

pub trait EmitStateChange {
    fn emit_clock_bank_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_clock_bank_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Clock(sc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_count_grows_shrinks_and_clamps() {
        let mut bank = ClockBank::new(4);
        assert_eq!(bank.len(), 4);
        assert_eq!(bank.as_static().len(), 4);

        bank.set_clock_count(8);
        assert_eq!(bank.len(), 8);
        assert_eq!(bank.as_static().len(), 8);

        bank.set_clock_count(4);
        assert_eq!(bank.len(), 4);

        // Requests beyond MAX_CLOCKS clamp rather than overflow.
        bank.set_clock_count(MAX_CLOCKS + 5);
        assert_eq!(bank.len(), MAX_CLOCKS);
        assert_eq!(ClockBank::new(MAX_CLOCKS + 5).len(), MAX_CLOCKS);
    }

    #[test]
    fn out_of_range_index_reads_are_neutral() {
        let bank = ClockBank::new(4);
        let missing = ClockIdx(9);
        assert!(bank.get(missing).is_none());
        assert_eq!(bank.phase(missing), Phase::ZERO);
        assert_eq!(bank.ticks(missing), 0);
        assert_eq!(bank.submaster_level(missing), UnipolarFloat::ZERO);
        assert!(!bank.use_audio_size(missing));
    }
}
