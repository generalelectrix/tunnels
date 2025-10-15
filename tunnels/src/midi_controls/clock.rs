//! Midi control declarations for clocks.

use crate::{
    clock::ControlMessage as ClockControlMessage,
    clock::StateChange as ClockStateChange,
    clock_bank::ClockIdxExt,
    clock_bank::ControlMessage,
    clock_bank::StateChange,
    clock_bank::N_CLOCKS,
    midi::{cc, event, note_on, Manager, Mapping},
    midi_controls::Device,
    midi_controls::{bipolar_to_midi, unipolar_to_midi},
    show::ControlMessage::Clock,
};

use super::{bipolar_from_midi, unipolar_from_midi, ControlMap};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Control {
    Rate,
    RateFine,
    Level,
    Tap,
    OneShot,
    Retrigger,
    AudioSize,
    AudioSpeed,
}

/// Return a control mapping for the CMD-MM1.
fn mapping_cmd_mm1(control: Control, channel: usize) -> Option<Mapping> {
    use Control::*;

    let channel = channel as u8;
    let midi_channel = 4;

    match control {
        Rate => Some(cc(midi_channel, 6 + channel)),
        RateFine => Some(cc(midi_channel, 18 + channel)),
        Level => Some(cc(midi_channel, 48 + channel)),
        Tap => Some(note_on(midi_channel, 48 + channel)),
        OneShot => Some(note_on(midi_channel, 19 + channel * 4)),
        Retrigger => Some(note_on(midi_channel, 20 + channel * 4)),
        AudioSize | AudioSpeed => None, // FIXME: not enough physical buttons
    }
}

/// Return a control mapping for TouchOSC.
fn mapping_touchosc(control: Control, channel: usize) -> Option<Mapping> {
    use Control::*;

    // lay out controls with same values, increment channels
    // start at a high channel where we have no existing mappings
    let channel = 9 + channel as u8;

    Some(match control {
        Rate => cc(channel, 0),
        RateFine => {
            return None;
        } // TODO: fine rate control on TouchOSC
        Level => cc(channel, 1),
        Tap => note_on(channel, 0),
        OneShot => note_on(channel, 1),
        Retrigger => note_on(channel, 2),
        AudioSize => note_on(channel, 3),
        AudioSpeed => note_on(channel, 4),
    })
}

pub fn map_clock_controls(device: Device, map: &mut ControlMap) {
    use ClockControlMessage::*;
    use ClockStateChange::*;

    let mut add = |mapping: Option<Mapping>, creator| {
        if let Some(mapping) = mapping {
            map.add(device, mapping, creator)
        }
    };

    let get_mapping = match device {
        Device::BehringerCmdMM1 => mapping_cmd_mm1,
        Device::TouchOsc => mapping_touchosc,
        _ => panic!("No clock control mappings for {device}."),
    };

    // This is to catch a future change to N_CLOCKS.
    #[allow(clippy::assertions_on_constants)]
    (assert!(N_CLOCKS <= 4, "The CMD MM-1 only has 4 channel rows."));

    for channel in 0..N_CLOCKS {
        add(
            get_mapping(Control::Rate, channel),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: Set(Rate(bipolar_from_midi(v))),
                })
            }),
        );
        add(
            get_mapping(Control::RateFine, channel),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: Set(RateFine(bipolar_from_midi(v))),
                })
            }),
        );
        add(
            get_mapping(Control::Level, channel),
            Box::new(move |v| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: Set(SubmasterLevel(unipolar_from_midi(v))),
                })
            }),
        );
        add(
            get_mapping(Control::Tap, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: Tap,
                })
            }),
        );
        add(
            get_mapping(Control::OneShot, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: ToggleOneShot,
                })
            }),
        );
        add(
            get_mapping(Control::Retrigger, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: Retrigger,
                })
            }),
        );
        add(
            get_mapping(Control::AudioSize, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: ToggleUseAudioSize,
                })
            }),
        );
        add(
            get_mapping(Control::AudioSpeed, channel),
            Box::new(move |_| {
                Clock(ControlMessage {
                    channel: ClockIdxExt(channel),
                    msg: ToggleUseAudioSpeed,
                })
            }),
        );
    }
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_clock_control(sc: StateChange, manager: &mut Manager<Device>) {
    use ClockStateChange::*;

    let mut send = |control, value| {
        if let Some(mapping) = mapping_cmd_mm1(control, sc.channel.into()) {
            manager.send(&Device::BehringerCmdMM1, event(mapping, value));
        }
        if let Some(mapping) = mapping_touchosc(control, sc.channel.into()) {
            manager.send(&Device::TouchOsc, event(mapping, value));
        }
    };

    match sc.change {
        OneShot(v) => send(Control::OneShot, v as u8),
        Ticked(v) => send(Control::Tap, v as u8),
        Rate(v) => send(Control::Rate, bipolar_to_midi(v)),
        RateFine(v) => send(Control::RateFine, bipolar_to_midi(v)),
        SubmasterLevel(v) => send(Control::Level, unipolar_to_midi(v)),
        UseAudioSize(v) => send(Control::AudioSize, v as u8),
        UseAudioSpeed(v) => send(Control::AudioSpeed, v as u8),
    }
}
