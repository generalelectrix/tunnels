use std::time::Duration;

use crate::{
    clock::{
        ControlMessage as ClockControlMessage, ControllableClock,
        EmitStateChange as EmitClockStateChange, StateChange as ClockStateChange,
    },
    master_ui::EmitStateChange as EmitShowStateChange,
};
use serde::{Deserialize, Serialize};
use tunnels_lib::number::UnipolarFloat;
use typed_index_derive::TypedIndex;

/// how many globally-available clocks?
pub const N_CLOCKS: usize = 4;

#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(ControllableClock)]
pub struct ClockIdx(pub usize);

/// Maintain a indexable collection of clocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockBank([ControllableClock; N_CLOCKS]);

impl ClockBank {
    pub fn new() -> Self {
        Self(Default::default())
    }

    /// Return an immutable reference to the selected clock.
    pub fn get(&self, index: ClockIdx) -> &ControllableClock {
        &self.0[index]
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
        self.0[msg.channel].control(
            msg.msg,
            &mut ChannelEmitter {
                channel: msg.channel,
                emitter,
            },
        )
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

pub struct ControlMessage {
    pub channel: ClockIdx,
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
