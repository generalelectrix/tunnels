//! Midi control declarations for clocks.

use crate::{
    clock::ControlMessage as ClockControlMessage,
    clock::StateChange as ClockStateChange,
    clock_bank::ClockIdx,
    clock_bank::ControlMessage,
    clock_bank::StateChange,
    clock_bank::N_CLOCKS,
    device::Device,
    midi::{cc, event, note_on, Manager},
    show::ControlMessage::Clock,
};

use super::{bipolar_from_midi, unipolar_from_midi, ControlMap};

const RATE_CH_0: u8 = 6;
const LEVEL_CH_0: u8 = 48;
const MIDI_CHANNEL: u8 = 4;
const TAP_CH_0: u8 = 48;

const ONESHOTS: [u8; N_CLOCKS] = [19, 23, 27, 31];
const RETRIGGERS: [u8; N_CLOCKS] = [20, 24, 28, 32];

const LED_OFF: u8 = 0;
const LED_ON: u8 = 1;
#[allow(unused)]
const LED_BLINK: u8 = 2;

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
        add(
            note_on(MIDI_CHANNEL, TAP_CH_0 + i as u8),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdx(i),
                    msg: Tap,
                })
            }),
        );
        add(
            note_on(MIDI_CHANNEL, ONESHOTS[i]),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdx(i),
                    msg: ToggleOneShot,
                })
            }),
        );
        add(
            note_on(MIDI_CHANNEL, RETRIGGERS[i]),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdx(i),
                    msg: ToggleRetrigger,
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

    match sc.change {
        Retrigger(v) => send(event(
            note_on(MIDI_CHANNEL, RETRIGGERS[sc.channel.0]),
            if v { LED_ON } else { LED_OFF },
        )),
        OneShot(v) => send(event(
            note_on(MIDI_CHANNEL, ONESHOTS[sc.channel.0]),
            if v { LED_ON } else { LED_OFF },
        )),
        Ticked(v) => send(event(
            note_on(MIDI_CHANNEL, TAP_CH_0 + sc.channel.0 as u8),
            if v { LED_ON } else { LED_OFF },
        )),
        Rate(_) | SubmasterLevel(_) => (),
    }
}
