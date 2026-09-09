use crate::{
    midi::{Event, EventType, MidiOutput, cc, event, note_on},
    midi_controls::Device,
    mixer::ControlMessage,
    mixer::StateChange,
    mixer::{
        ChannelControlMessage, ChannelIdx, ChannelStateChange, Mixer,
        VideoChannel as VideoChannelIdx,
    },
    show::ControlMessage as ShowControlMessage,
};
use tunnels_model::layer::PaintMode;

use super::{unipolar_from_midi, unipolar_to_midi};

/// The number of mixer channels on a single mixer page.
///
/// The mixer sizes itself in whole pages, so the page size lives with the
/// mixer rather than with the control surface that mirrors it.
pub use crate::mixer::MIXER_CHANNELS_PER_PAGE as PAGE_SIZE;

const FADER: u8 = 0x7;
const MASK: u8 = 0x31;
const LOOK: u8 = 0x30;

/// The note a channel's bump button sends.
///
/// The APC40's Clip Stop, which is a larger button than the row bump used to
/// sit on and so an easier one to mash mid-show.
///
/// **Bump is the one control here that is momentary**: the note on sets it and
/// the note off releases it. That makes this the note whose behaviour on
/// release matters, and the first thing to check with the desk in front of
/// you. A button that sends only on press leaves bump latched on, which is a
/// channel stuck at full for as long as nobody notices — a worse failure than
/// any of the toggles above can produce, and one that looks like a fader
/// problem rather than a note problem.
const BUMP: u8 = 0x34;

/// The note a channel's gobo button sends, and lights its lamp from.
///
/// The APC40's Activator, which the row above bump used to occupy and which
/// bump moving to Clip Stop freed. Its lamp is green, so a gobo'd channel
/// reads green against a mask's blue, and the look indicator keeps red to
/// match the saved-look red in the beam grid.
///
/// **Taken from the protocol document rather than measured off the hardware**,
/// so a surface that turns out to send something else here needs this number
/// changed and nothing else.
const GOBO: u8 = 0x32;

/// The midi note value for the 0th video channel selector.
const VIDEO_CHAN_0: u8 = 66;

pub fn interpret(event: &Event, page: usize) -> Option<ShowControlMessage> {
    use ChannelControlMessage::*;
    use ChannelStateChange::*;

    let channel_offset = page * PAGE_SIZE;
    let chan = event.mapping.channel as usize;
    if chan >= PAGE_SIZE {
        return None;
    }
    let v = event.value;
    let mkmsg = |ccm: ChannelControlMessage| -> ShowControlMessage {
        ShowControlMessage::Mixer(ControlMessage {
            channel: ChannelIdx(chan + channel_offset),
            msg: ccm,
        })
    };

    let control = event.mapping.control;
    Some(match event.mapping.event_type {
        EventType::ControlChange if control == FADER => mkmsg(Set(Level(unipolar_from_midi(v)))),
        EventType::NoteOn if control == BUMP => mkmsg(Set(Bump(true))),
        EventType::NoteOff if control == BUMP => mkmsg(Set(Bump(false))),
        EventType::NoteOn if control == MASK => mkmsg(ToggleMode(PaintMode::Mask)),
        EventType::NoteOn if control == GOBO => mkmsg(ToggleMode(PaintMode::Gobo)),
        EventType::NoteOn
            if control >= VIDEO_CHAN_0
                && control < VIDEO_CHAN_0 + Mixer::N_VIDEO_CHANNELS as u8 =>
        {
            let vc = (control - VIDEO_CHAN_0) as usize;
            mkmsg(ToggleVideoChannel(VideoChannelIdx(vc)))
        }
        _ => return None,
    })
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_mixer_control(sc: StateChange, manager: &mut impl MidiOutput) {
    use ChannelStateChange::*;

    let page = sc.channel.0 / PAGE_SIZE;
    let channel_offset = page * PAGE_SIZE;
    let midi_channel = (sc.channel.0 - channel_offset) as u8;

    let mut send = |event| {
        // Send page 0 to the APC40, page 1 to APC20
        manager.send(
            if page == 0 {
                &Device::AkaiApc40
            } else {
                &Device::AkaiApc20
            },
            event,
        );
        manager.send(&Device::TouchOsc, event);
    };

    match sc.change {
        Level(v) => send(event(cc(midi_channel, FADER), unipolar_to_midi(v))),
        Bump(v) => send(event(note_on(midi_channel, BUMP), v as u8)),
        // A mode lights one lamp and darkens the other, so a channel's mode
        // is legible from the surface whichever of the two it is in.
        Mode(v) => {
            send(event(
                note_on(midi_channel, MASK),
                (v == PaintMode::Mask) as u8,
            ));
            send(event(
                note_on(midi_channel, GOBO),
                (v == PaintMode::Gobo) as u8,
            ));
        }
        ContainsLook(v) => send(event(note_on(midi_channel, LOOK), v as u8)),
        VideoChannel((vc, v)) => send(event(
            note_on(midi_channel, vc.0 as u8 + VIDEO_CHAN_0),
            v as u8,
        )),
    }
}
