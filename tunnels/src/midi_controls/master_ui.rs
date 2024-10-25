use super::{mixer::PAGE_SIZE, ControlMap, RadioButtons};
use crate::{
    beam_store::{BeamStore, BeamStoreAddr},
    master_ui::ControlMessage,
    master_ui::StateChange,
    master_ui::{BeamButtonState, BeamStoreState as BeamStoreStatePayload},
    midi::{event, note_on, note_on_ch0, Manager, Mapping},
    midi_controls::Device,
    mixer::ChannelIdx,
    show::ControlMessage::MasterUI,
    tunnel::{AnimationIdx, N_ANIM},
};
use lazy_static::lazy_static;

const CHANNEL_SELECT: u8 = 0x33;
const ANIM_0_BUTTON: u8 = 0x57;
const ANIM_COPY: Mapping = note_on_ch0(0x65);
const ANIM_PASTE: Mapping = note_on_ch0(0x64);

const BEAM_SAVE: Mapping = note_on_ch0(0x52);
const LOOK_SAVE: Mapping = note_on_ch0(0x53);
const BEAM_DELETE: Mapping = note_on_ch0(0x54);
const LOOK_EDIT: Mapping = note_on_ch0(0x56);

const BEAM_GRID_ROW_0: u8 = 0x35;

// APC40 main button grid LED states
const LED_OFF: u8 = 0;
#[allow(unused)]
const LED_SOLID_GREEN: u8 = 1;
#[allow(unused)]
const LED_BLINK_GREEN: u8 = 2;
const LED_SOLID_RED: u8 = 3;
#[allow(unused)]
const LED_BLINK_RED: u8 = 4;
const LED_SOLID_ORANGE: u8 = 5;
#[allow(unused)]
const LED_BLINK_ORANGE: u8 = 6;

lazy_static! {
    static ref ANIMATION_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: (0..N_ANIM)
            .map(|aid| note_on_ch0(aid as u8 + ANIM_0_BUTTON))
            .collect(),
        off: 0,
        on: 1,
    };
    static ref CHANNEL_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: (0..PAGE_SIZE)
            .map(|cid| note_on(cid as u8, CHANNEL_SELECT))
            .collect(),
        off: 0,
        on: 1,
    };
    static ref BEAM_STORE_STATE_BUTTONS: RadioButtons = RadioButtons {
        mappings: vec!(BEAM_SAVE, LOOK_SAVE, BEAM_DELETE, LOOK_EDIT),
        off: 0,
        on: 2,
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
    add(ANIM_COPY, Box::new(|_| MasterUI(AnimationCopy)));
    add(ANIM_PASTE, Box::new(|_| MasterUI(AnimationPaste)));
    add(
        BEAM_SAVE,
        Box::new(|_| MasterUI(Set(BeamStoreState(BeamStoreStatePayload::BeamSave)))),
    );
    add(
        LOOK_SAVE,
        Box::new(|_| MasterUI(Set(BeamStoreState(BeamStoreStatePayload::LookSave)))),
    );
    add(
        BEAM_DELETE,
        Box::new(|_| MasterUI(Set(BeamStoreState(BeamStoreStatePayload::Delete)))),
    );
    add(
        LOOK_EDIT,
        Box::new(|_| MasterUI(Set(BeamStoreState(BeamStoreStatePayload::LookEdit)))),
    );

    let col_offset = BeamStore::COLS_PER_PAGE * page;
    for row in 0..BeamStore::N_ROWS {
        for col in 0..BeamStore::COLS_PER_PAGE {
            add(
                note_on(col as u8, row as u8 + BEAM_GRID_ROW_0),
                Box::new(move |_| {
                    MasterUI(BeamGridButtonPress(BeamStoreAddr {
                        row,
                        col: col + col_offset,
                    }))
                }),
            )
        }
    }
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_master_ui_control(sc: StateChange, manager: &mut Manager<Device>) {
    use StateChange::*;

    let mut send_main = |event| {
        manager.send(Device::TouchOsc, event);
        manager.send(Device::AkaiApc40, event);
    };

    match sc {
        Animation(a) => {
            ANIMATION_SELECT_BUTTONS.select(note_on_ch0(ANIM_0_BUTTON + a.0 as u8), send_main);
        }
        Channel(c) => {
            let page = c.0 / PAGE_SIZE;
            let channel_offset = page * PAGE_SIZE;
            let midi_channel = (c.0 - channel_offset) as u8;

            // Send to the appropriate device based on page.
            // If this channel is on page 0, disable all channel buttons on APC20.
            // If page 1, disable all buttons on APC40/TouchOSC.
            if page == 0 {
                CHANNEL_SELECT_BUTTONS.select(note_on(midi_channel, CHANNEL_SELECT), send_main);
                CHANNEL_SELECT_BUTTONS.all_off(|event| manager.send(Device::AkaiApc20, event));
            } else {
                CHANNEL_SELECT_BUTTONS.all_off(send_main);
                CHANNEL_SELECT_BUTTONS.select(note_on(midi_channel, CHANNEL_SELECT), |event| {
                    manager.send(Device::AkaiApc20, event)
                });
            }
        }
        BeamButton((addr, state)) => {
            let page = addr.col / BeamStore::COLS_PER_PAGE;
            let col_offset = page * BeamStore::COLS_PER_PAGE;
            let midi_channel = (addr.col - col_offset) as u8;

            use BeamButtonState::*;
            let e = event(
                note_on(midi_channel, BEAM_GRID_ROW_0 + addr.row as u8),
                match state {
                    Empty => LED_OFF,
                    Beam => LED_SOLID_ORANGE,
                    Look => LED_SOLID_RED,
                },
            );

            if page == 0 {
                send_main(e);
            } else {
                manager.send(Device::AkaiApc20, e);
            }
        }
        BeamStoreState(state) => {
            let send_all = |event| {
                manager.send(Device::TouchOsc, event);
                manager.send(Device::AkaiApc40, event);
                manager.send(Device::AkaiApc20, event);
            };
            use BeamStoreStatePayload::*;
            match state {
                Idle => BEAM_STORE_STATE_BUTTONS.all_off(send_all),
                BeamSave => BEAM_STORE_STATE_BUTTONS.select(BEAM_SAVE, send_all),
                LookSave => BEAM_STORE_STATE_BUTTONS.select(LOOK_SAVE, send_all),
                Delete => BEAM_STORE_STATE_BUTTONS.select(BEAM_DELETE, send_all),
                LookEdit => BEAM_STORE_STATE_BUTTONS.select(LOOK_EDIT, send_all),
            }
        }
    }
}
