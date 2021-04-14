use crate::{
    clock::ClockIdx,
    device::Device,
    midi::{cc, event, note_off, note_on, Manager, Mapping},
    mixer::ControlMessage,
    mixer::StateChange,
    mixer::{
        ChannelControlMessage, ChannelIdx, ChannelStateChange, Mixer,
        VideoChannel as VideoChannelIdx,
    },
    show::ControlMessage as ShowControlMessage,
};

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap,
    RadioButtons,
};

const FADER: u8 = 0x7;
const BUMP: u8 = 0x32;
const MASK: u8 = 0x31;
const LOOK: u8 = 0x30;

/// The midi note value for the 0th video channel selector.
const VIDEO_CHAN_0: u8 = 66;

/// The number of mixer channels on a single mixer page.
const PAGE_SIZE: usize = 8;

pub fn map_mixer_controls(device: Device, page: usize, map: &mut ControlMap) {
    use ChannelControlMessage::*;
    use ChannelStateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    // Offset the mixer channels to correspond to this page.
    let channel_offset = page * PAGE_SIZE;

    for chan in 0..PAGE_SIZE {
        let mkmsg = move |ccm: ChannelControlMessage| -> ShowControlMessage {
            ShowControlMessage::Mixer(ControlMessage::Channel((
                ChannelIdx(chan + channel_offset),
                ccm,
            )))
        };
        add(
            cc(chan as u8, FADER),
            Box::new(move |v| mkmsg(Set(Level(unipolar_from_midi(v))))),
        );
        add(
            note_on(chan as u8, BUMP),
            Box::new(move |_| mkmsg(Set(Bump(true)))),
        );
        add(
            note_off(chan as u8, BUMP),
            Box::new(move |_| mkmsg(Set(Bump(false)))),
        );
        add(
            note_on(chan as u8, MASK),
            Box::new(move |_| mkmsg(ToggleMask)),
        );

        // Configure the video channel selectors.
        for vc in 0..Mixer::N_VIDEO_CHANNELS {
            add(
                note_on(chan as u8, vc as u8 + VIDEO_CHAN_0),
                Box::new(move |_| mkmsg(ToggleVideoChannel(VideoChannelIdx(vc)))),
            );
        }
    }
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_mixer_control(sc: StateChange, manager: &mut Manager) {
    use ChannelStateChange::*;

    let page = sc.channel.0 / PAGE_SIZE;
    let channel_offset = page * PAGE_SIZE;
    let midi_channel = (sc.channel.0 - channel_offset) as u8;

    let mut send = |event| {
        // Send page 0 to the APC40, page 1 to APC20
        manager.send(
            if page == 0 {
                Device::AkaiApc40
            } else {
                Device::AkaiApc20
            },
            event,
        );
        manager.send(Device::TouchOsc, event);
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
