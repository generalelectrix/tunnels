//! Midi control declarations for clocks.

use super::{bipolar_from_midi, unipolar_from_midi};
use crate::midi::Event as MidiEvent;
use crate::{
    clock::ControlMessage as ClockControlMessage,
    clock::StateChange as ClockStateChange,
    clock_bank::ClockIdxExt,
    clock_bank::ControlMessage,
    clock_bank::StateChange,
    clock_bank::N_CLOCKS,
    midi::{cc, event, note_on, Mapping, MidiOutput},
    midi_controls::Device,
    midi_controls::{bipolar_to_midi, unipolar_to_midi},
};

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

fn interpret_with_mapping_fn(
    event: &MidiEvent,
    get_mapping: fn(Control, usize) -> Option<Mapping>,
) -> Option<crate::show::ControlMessage> {
    use ClockControlMessage::*;
    use ClockStateChange::*;
    let v = event.value;

    for channel in 0..N_CLOCKS {
        let mkmsg = |msg| {
            crate::show::ControlMessage::Clock(ControlMessage {
                channel: ClockIdxExt(channel),
                msg,
            })
        };

        if get_mapping(Control::Rate, channel) == Some(event.mapping) {
            return Some(mkmsg(Set(Rate(bipolar_from_midi(v)))));
        }
        if get_mapping(Control::RateFine, channel) == Some(event.mapping) {
            return Some(mkmsg(Set(RateFine(bipolar_from_midi(v)))));
        }
        if get_mapping(Control::Level, channel) == Some(event.mapping) {
            return Some(mkmsg(Set(SubmasterLevel(unipolar_from_midi(v)))));
        }
        if get_mapping(Control::Tap, channel) == Some(event.mapping) {
            return Some(mkmsg(Tap));
        }
        if get_mapping(Control::OneShot, channel) == Some(event.mapping) {
            return Some(mkmsg(ToggleOneShot));
        }
        if get_mapping(Control::Retrigger, channel) == Some(event.mapping) {
            return Some(mkmsg(Retrigger));
        }
        if get_mapping(Control::AudioSize, channel) == Some(event.mapping) {
            return Some(mkmsg(ToggleUseAudioSize));
        }
        if get_mapping(Control::AudioSpeed, channel) == Some(event.mapping) {
            return Some(mkmsg(ToggleUseAudioSpeed));
        }
    }
    None
}

pub fn interpret_touchosc(event: &MidiEvent) -> Option<crate::show::ControlMessage> {
    interpret_with_mapping_fn(event, mapping_touchosc)
}

pub fn interpret_cmdmm1(event: &MidiEvent) -> Option<crate::show::ControlMessage> {
    interpret_with_mapping_fn(event, mapping_cmd_mm1)
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_clock_control(sc: StateChange, manager: &mut impl MidiOutput) {
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
