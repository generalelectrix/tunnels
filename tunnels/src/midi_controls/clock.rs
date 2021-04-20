#![allow(unused)]
use crate::{
    clock::ControlMessage as ClockControlMessage,
    clock::StateChange as ClockStateChange,
    clock_bank::ClockIdx,
    clock_bank::ControlMessage,
    clock_bank::StateChange,
    clock_bank::N_CLOCKS,
    device::Device,
    midi::{cc, event, note_off, note_on, Manager, Mapping},
    show::ControlMessage::Clock,
};

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap,
    RadioButtons,
};

const RATE_CH_0: u8 = 6;
const LEVEL_CH_0: u8 = 48;
const MIDI_CHANNEL: u8 = 4;

pub fn map_clock_controls(device: Device, map: &mut ControlMap) {
    use ClockControlMessage::*;
    use ClockStateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    assert!(N_CLOCKS <= 4, "The CMD MM-1 only has 4 channel rows.");
    for i in 0..N_CLOCKS {
        add(
            cc(MIDI_CHANNEL, RATE_CH_0 + i as u8),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdx(i),
                    msg: Set(Rate(bipolar_from_midi(v))),
                })
            }),
        );
        add(
            cc(MIDI_CHANNEL, LEVEL_CH_0 + i as u8),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdx(i),
                    msg: Set(SubmasterLevel(unipolar_from_midi(v))),
                })
            }),
        );
    }
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_clock_control(sc: StateChange, manager: &mut Manager) {
    use ClockStateChange::*;

    let mut send = |event| {
        manager.send(Device::BehringerCmdMM1, event);
    };
}
