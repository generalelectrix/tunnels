//! Midi control declarations for clocks.

use super::{bipolar_from_midi, unipolar_from_midi};
use crate::midi::Event as MidiEvent;
use crate::{
    clock::ControlMessage as ClockControlMessage,
    clock::StateChange as ClockStateChange,
    clock_bank::CLOCKS_PER_WING,
    clock_bank::ClockIdx,
    clock_bank::ControlMessage,
    clock_bank::StateChange,
    midi::{Mapping, MidiOutput, cc, event, note_on},
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

/// TouchOSC clock control maps one MIDI channel per clock starting at channel 9,
/// so it can only address clocks that fit within the MIDI channel range. The
/// primary clock surface is the CMD MM-1 wings.
const TOUCHOSC_CLOCK_CHANNELS: usize = 4;

fn interpret_with_mapping_fn(
    event: &MidiEvent,
    get_mapping: fn(Control, usize) -> Option<Mapping>,
    channel_offset: usize,
    n_channels: usize,
) -> Option<crate::show::ControlMessage> {
    use ClockControlMessage::*;
    use ClockStateChange::*;
    let v = event.value;

    for local in 0..n_channels {
        let mkmsg = |msg| {
            crate::show::ControlMessage::Clock(ControlMessage {
                channel: ClockIdx(local + channel_offset),
                msg,
            })
        };

        if get_mapping(Control::Rate, local) == Some(event.mapping) {
            return Some(mkmsg(Set(Rate(bipolar_from_midi(v)))));
        }
        if get_mapping(Control::RateFine, local) == Some(event.mapping) {
            return Some(mkmsg(Set(RateFine(bipolar_from_midi(v)))));
        }
        if get_mapping(Control::Level, local) == Some(event.mapping) {
            return Some(mkmsg(Set(SubmasterLevel(unipolar_from_midi(v)))));
        }
        if get_mapping(Control::Tap, local) == Some(event.mapping) {
            return Some(mkmsg(Tap));
        }
        if get_mapping(Control::OneShot, local) == Some(event.mapping) {
            return Some(mkmsg(ToggleOneShot));
        }
        if get_mapping(Control::Retrigger, local) == Some(event.mapping) {
            return Some(mkmsg(Retrigger));
        }
        if get_mapping(Control::AudioSize, local) == Some(event.mapping) {
            return Some(mkmsg(ToggleUseAudioSize));
        }
        if get_mapping(Control::AudioSpeed, local) == Some(event.mapping) {
            return Some(mkmsg(ToggleUseAudioSpeed));
        }
    }
    None
}

pub fn interpret_touchosc(event: &MidiEvent) -> Option<crate::show::ControlMessage> {
    interpret_with_mapping_fn(event, mapping_touchosc, 0, TOUCHOSC_CLOCK_CHANNELS)
}

/// Interpret a CMD MM-1 clock-wing event, mapping its physical faders to the
/// clocks starting at `channel_offset`.
pub fn interpret_cmdmm1(
    event: &MidiEvent,
    channel_offset: usize,
) -> Option<crate::show::ControlMessage> {
    interpret_with_mapping_fn(event, mapping_cmd_mm1, channel_offset, CLOCKS_PER_WING)
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_clock_control(sc: StateChange, manager: &mut impl MidiOutput) {
    use ClockStateChange::*;

    // Route feedback to the wing that owns this clock, at its local fader index.
    let global = sc.channel.0;
    let wing_offset = (global / CLOCKS_PER_WING) * CLOCKS_PER_WING;
    let local = global % CLOCKS_PER_WING;

    let mut send = |control, value| {
        if let Some(mapping) = mapping_cmd_mm1(control, local) {
            manager.send(
                &Device::BehringerCmdMM1 {
                    channel_offset: wing_offset,
                },
                event(mapping, value),
            );
        }
        if global < TOUCHOSC_CLOCK_CHANNELS
            && let Some(mapping) = mapping_touchosc(control, global)
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmdmm1_wing_offset_selects_the_right_clock() {
        // The Rate control for a wing's first fader is cc(4, 6).
        let event = MidiEvent {
            mapping: cc(4, 6),
            value: 64,
        };
        for (offset, expected) in [(0usize, 0usize), (4, 4), (8, 8)] {
            match interpret_cmdmm1(&event, offset) {
                Some(crate::show::ControlMessage::Clock(ControlMessage { channel, .. })) => {
                    assert_eq!(channel.0, expected, "wing offset {offset}");
                }
                other => {
                    panic!("expected a clock control message for offset {offset}, got {other:?}")
                }
            }
        }
    }
}
