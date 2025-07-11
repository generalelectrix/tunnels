use std::time::Duration;

use crate::{
    clock::{
        ControlMessage as ClockControlMessage, ControllableClock,
        EmitStateChange as EmitClockStateChange, StateChange as ClockStateChange, StaticClock,
        Ticks,
    },
    master_ui::EmitStateChange as EmitShowStateChange,
};
use anyhow::{bail, Error, Result};
use log::error;
use serde::{Deserialize, Serialize};
use tunnels_lib::number::{Phase, UnipolarFloat};
use typed_index_derive::TypedIndex;

/// Read-only interface to the state of a collection of clocks.
pub trait ClockStore {
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

/// how many globally-available clocks?
pub const N_CLOCKS: usize = 4;

#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(ControllableClock)]
/// Index of a clock in the bank.
/// Care should be taken to ensure that these values are always valid.
/// External input should be accepted through ClockIdxExt and validated
/// using from.
pub struct ClockIdx(usize);

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
/// Public-facing "request" for a clock index.
/// Must be validated to become a proper ClockIdx.
pub struct ClockIdxExt(pub usize);

impl TryFrom<ClockIdxExt> for ClockIdx {
    type Error = Error;
    fn try_from(value: ClockIdxExt) -> Result<Self> {
        if value.0 >= N_CLOCKS {
            bail!("clock index {} out of range", value.0);
        }
        Ok(ClockIdx(value.0))
    }
}

/// Maintain a indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClockBank([ControllableClock; N_CLOCKS]);

impl ClockStore for ClockBank {
    fn phase(&self, index: ClockIdx) -> Phase {
        self.get(index).phase()
    }

    fn ticks(&self, index: ClockIdx) -> Ticks {
        self.get(index).ticks()
    }

    fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat {
        self.get(index).submaster_level()
    }

    fn use_audio_size(&self, index: ClockIdx) -> bool {
        self.get(index).use_audio_size()
    }
}

impl ClockBank {
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

    pub fn get(&self, index: ClockIdx) -> &ControllableClock {
        &self.0[index]
    }

    /// Return a static snapshot of the state of this clock bank.
    pub fn as_static(&self) -> [StaticClock; N_CLOCKS] {
        // FIXME: need this method to avoid having to write this out explicitly
        // https://doc.rust-lang.org/std/primitive.array.html#method.each_ref
        [
            self.0[0].as_static(),
            self.0[1].as_static(),
            self.0[2].as_static(),
            self.0[3].as_static(),
        ]
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

#[derive(Debug, Clone)]
pub struct ControlMessage {
    pub channel: ClockIdxExt,
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
