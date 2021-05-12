//! Midi control declarations for clocks.

use crate::{
    clock::ControlMessage as ClockControlMessage,
    clock::StateChange as ClockStateChange,
    clock_bank::ClockIdx,
    clock_bank::ControlMessage,
    clock_bank::StateChange,
    clock_bank::N_CLOCKS,
    device::Device,
    midi::{cc, event, note_on, Manager, Mapping},
    show::ControlMessage::Clock,
};

use super::{bipolar_from_midi, unipolar_from_midi, ControlMap};

#[derive(PartialEq, Eq, Hash)]
enum Control {
    Rate,
    Level,
    Tap,
    OneShot,
    Retrigger,
}

/// Return a control mapping for the CMD-MM1.
fn mapping_cmd_mm1(control: Control, channel: usize) -> Mapping {
    use Control::*;

    let channel = channel as u8;
    let midi_channel = 4;

    match control {
        Rate => cc(midi_channel, 6 + channel),
        Level => cc(midi_channel, 48 + channel),
        Tap => note_on(midi_channel, 48 + channel),
        OneShot => note_on(midi_channel, 19 + channel * 4),
        Retrigger => note_on(midi_channel, 20 + channel * 4),
    }
}

pub fn map_clock_controls(device: Device, map: &mut ControlMap) {
    use ClockControlMessage::*;
    use ClockStateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    let get_mapping = match device {
        Device::BehringerCmdMM1 => mapping_cmd_mm1,
        _ => panic!("No clock control mappings for {}.", device),
    };

    for channel in 0..N_CLOCKS {
        if device == Device::BehringerCmdMM1 {
            assert!(channel < 4, "The CMD MM-1 only has 4 channel rows.");
        }
        add(
            get_mapping(Control::Rate, channel),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdx(channel),
                    msg: Set(Rate(bipolar_from_midi(v))),
                })
            }),
        );
        add(
            get_mapping(Control::Level, channel),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdx(channel),
                    msg: Set(SubmasterLevel(unipolar_from_midi(v))),
                })
            }),
        );
        add(
            get_mapping(Control::Tap, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdx(channel),
                    msg: Tap,
                })
            }),
        );
        add(
            get_mapping(Control::OneShot, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdx(channel),
                    msg: ToggleOneShot,
                })
            }),
        );
        add(
            get_mapping(Control::Retrigger, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdx(channel),
                    msg: ToggleRetrigger,
                })
            }),
        );
    }
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_clock_control(sc: StateChange, manager: &mut Manager) {
    use ClockStateChange::*;

    let mut send = |control, value| {
        manager.send(
            Device::BehringerCmdMM1,
            event(mapping_cmd_mm1(control, sc.channel.0), value),
        );
    };

    match sc.change {
        Retrigger(v) => send(Control::Retrigger, v as u8),
        OneShot(v) => send(Control::OneShot, v as u8),
        Ticked(v) => send(Control::Tap, v as u8),
        Rate(_) | SubmasterLevel(_) => (),
    }
}
