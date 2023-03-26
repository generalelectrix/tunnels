use std::time::Duration;

use crate::{
    clock::{
        Clock, ClockState, ControlMessage as ClockControlMessage, ControllableClock,
        EmitStateChange as EmitClockStateChange, StateChange as ClockStateChange,
    },
    master_ui::EmitStateChange as EmitShowStateChange,
};
use log::error;
use serde::{Deserialize, Serialize};
use simple_error::{bail, SimpleError};
use tunnels_lib::number::UnipolarFloat;
use typed_index_derive::TypedIndex;

/// Read-only interface to a store of clocks, accessed via the ClockState trait.
pub trait ClockStore {
    /// Access a clock's state by index.
    /// Return None if the index is out of bounds.
    fn get(&self, index: ClockIdx) -> &dyn ClockState;
}

/// how many globally-available clocks?
pub const N_CLOCKS: usize = 4;

#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(ControllableClock)]
/// Index of a clock in the bank.
/// Validated to always be in-range.
pub struct ClockIdx(usize);

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
/// Public-facing "request" for a clock index.
/// Must be validated to become a proper ClockIdx.
pub struct ClockIdxExt(pub usize);

impl TryFrom<ClockIdxExt> for ClockIdx {
    type Error = SimpleError;
    fn try_from(value: ClockIdxExt) -> Result<Self, Self::Error> {
        if value.0 >= N_CLOCKS {
            bail!("clock index {} out of range", value.0);
        }
        Ok(ClockIdx(value.0))
    }
}

/// Maintain a indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockBank([ControllableClock; N_CLOCKS]);

impl ClockStore for ClockBank {
    fn get(&self, index: ClockIdx) -> &dyn ClockState {
        &self.0[index] as &dyn ClockState
    }
}

impl ClockBank {
    pub fn new() -> Self {
        Self(Default::default())
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

    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        for (i, clock) in self.0.iter().enumerate() {
            clock.emit_state(&mut ChannelEmitter {
                channel: ClockIdx(i),
                emitter,
            });
        }
    }

    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        let channel: ClockIdx = match msg.channel.try_into() {
            Ok(id) => id,
            Err(e) => {
                error!("could not process clock control message {msg:?}: {e}");
                return;
            }
        };
        self.0[channel].control(msg.msg, &mut ChannelEmitter { channel, emitter })
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

#[derive(Debug)]
pub struct ControlMessage {
    pub channel: ClockIdxExt,
    pub msg: ClockControlMessage,
}

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
