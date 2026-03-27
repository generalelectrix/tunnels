use crate::{
    midi::{cc, event, note_on, Event, EventType, MidiOutput},
    midi_controls::Device,
    mixer::ControlMessage,
    mixer::StateChange,
    mixer::{
        ChannelControlMessage, ChannelIdx, ChannelStateChange, Mixer,
        VideoChannel as VideoChannelIdx,
    },
    show::ControlMessage as ShowControlMessage,
};

use super::{unipolar_from_midi, unipolar_to_midi};

const FADER: u8 = 0x7;
const BUMP: u8 = 0x32;
const MASK: u8 = 0x31;
const LOOK: u8 = 0x30;

/// The midi note value for the 0th video channel selector.
const VIDEO_CHAN_0: u8 = 66;

/// The number of mixer channels on a single mixer page.
pub const PAGE_SIZE: usize = 8;

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
        EventType::NoteOn if control == MASK => mkmsg(ToggleMask),
        EventType::NoteOn if control >= VIDEO_CHAN_0 && control < VIDEO_CHAN_0 + Mixer::N_VIDEO_CHANNELS as u8 => {
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
        Mask(v) => send(event(note_on(midi_channel, MASK), v as u8)),
        ContainsLook(v) => send(event(note_on(midi_channel, LOOK), v as u8)),
        VideoChannel((vc, v)) => send(event(
            note_on(midi_channel, vc.0 as u8 + VIDEO_CHAN_0),
            v as u8,
        )),
    }
}
