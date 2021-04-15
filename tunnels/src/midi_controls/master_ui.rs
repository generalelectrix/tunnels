use super::{
    bipolar_from_midi, bipolar_to_midi, mixer::PAGE_SIZE, unipolar_from_midi, unipolar_to_midi,
    ControlMap, RadioButtons,
};
use crate::{
    device::Device,
    master_ui::ControlMessage,
    master_ui::StateChange,
    midi::{cc, event, note_on, note_on_ch0, Manager, Mapping},
    mixer::ChannelIdx,
    show::ControlMessage as ShowControlMessage,
    show::ControlMessage::MasterUI,
    tunnel::{AnimationIdx, N_ANIM},
};
use lazy_static::lazy_static;

const CHANNEL_SELECT: u8 = 0x33;
const ANIM_0_BUTTON: u8 = 0x57;
const ANIM_COPY: u8 = 0x65;
const ANIM_PASTE: u8 = 0x64;

lazy_static! {
    static ref animation_select_buttons: RadioButtons = RadioButtons {
        mappings: (0..N_ANIM)
            .map(|aid| note_on_ch0(aid as u8 + ANIM_0_BUTTON))
            .collect(),
    };
    static ref channel_select_buttons: RadioButtons = RadioButtons {
        mappings: (0..PAGE_SIZE)
            .map(|cid| cc(cid as u8, CHANNEL_SELECT))
            .collect(),
    };
}

pub fn map_master_ui_controls(device: Device, page: usize, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;

    let channel_offset = page * PAGE_SIZE;

    let mut add = |mapping, creator| map.add(device, mapping, creator);
    for aid in 0..N_ANIM {
        add(
            note_on_ch0(ANIM_0_BUTTON + aid as u8),
            Box::new(move |_| MasterUI(Set(Animation(AnimationIdx(aid))))),
        );
    }
    for cid in 0..PAGE_SIZE {
        add(
            note_on(cid as u8, CHANNEL_SELECT),
            Box::new(move |_| MasterUI(Set(Channel(ChannelIdx(cid + channel_offset))))),
        );
    }
    add(
        note_on_ch0(ANIM_COPY),
        Box::new(|_| MasterUI(AnimationCopy)),
    );
    add(
        note_on_ch0(ANIM_PASTE),
        Box::new(|_| MasterUI(AnimationPaste)),
    );
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_master_ui_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(Device::TouchOsc, event);
        manager.send(Device::AkaiApc40, event);
    };

    match sc {
        Animation(a) => {
            animation_select_buttons.select(note_on_ch0(ANIM_0_BUTTON + a.0 as u8), send);
        }
        Channel(c) => {
            let page = c.0 / PAGE_SIZE;
            let channel_offset = page * PAGE_SIZE;
            let midi_channel = (c.0 - channel_offset) as u8;

            // Send to the appropriate device based on page.
            // If this channel is on page 0, disable all channel buttons on APC20.
            // If page 1, disable all buttons on APC40/TouchOSC.
            if page == 0 {
                channel_select_buttons.select(note_on(midi_channel, CHANNEL_SELECT), send);
                channel_select_buttons.all_off(|event| {
                    manager.send(Device::AkaiApc20, event);
                })
            } else {
                channel_select_buttons.all_off(send);
                channel_select_buttons.select(note_on(midi_channel, CHANNEL_SELECT), |event| {
                    manager.send(Device::AkaiApc20, event);
                })
            }
        }
    }
}
