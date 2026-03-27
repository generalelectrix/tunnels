use super::{mixer::PAGE_SIZE, RadioButtons};
use crate::{
    beam_store::{BeamStore, BeamStoreAddr},
    master_ui::ControlMessage,
    master_ui::StateChange,
    master_ui::{BeamButtonState, BeamStoreState as BeamStoreStatePayload},
    midi::{event, note_on, note_on_ch0, Event, EventType, MidiOutput, Mapping},
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

pub fn interpret(event: &Event, page: usize) -> Option<crate::show::ControlMessage> {
    use ControlMessage::*;
    use StateChange::*;

    let channel_offset = page * PAGE_SIZE;
    let m = event.mapping;

    // Animation select buttons (channel 0, NoteOn)
    if m.event_type == EventType::NoteOn
        && m.channel == 0
        && m.control >= ANIM_0_BUTTON
        && m.control < ANIM_0_BUTTON + N_ANIM as u8
    {
        let aid = (m.control - ANIM_0_BUTTON) as usize;
        return Some(MasterUI(Set(Animation(AnimationIdx(aid)))));
    }

    // Channel select buttons (per-channel NoteOn)
    if m.event_type == EventType::NoteOn
        && m.control == CHANNEL_SELECT
        && (m.channel as usize) < PAGE_SIZE
    {
        let cid = m.channel as usize;
        return Some(MasterUI(Set(Channel(ChannelIdx(cid + channel_offset)))));
    }

    // Beam grid buttons
    if m.event_type == EventType::NoteOn
        && m.control >= BEAM_GRID_ROW_0
        && m.control < BEAM_GRID_ROW_0 + BeamStore::N_ROWS as u8
        && (m.channel as usize) < BeamStore::COLS_PER_PAGE
    {
        let col_offset = BeamStore::COLS_PER_PAGE * page;
        let row = (m.control - BEAM_GRID_ROW_0) as usize;
        let col = m.channel as usize + col_offset;
        return Some(MasterUI(BeamGridButtonPress(BeamStoreAddr { row, col })));
    }

    Some(match m {
        _ if m == ANIM_COPY => MasterUI(AnimationCopy),
        _ if m == ANIM_PASTE => MasterUI(AnimationPaste),
        _ if m == BEAM_SAVE => MasterUI(Set(BeamStoreState(BeamStoreStatePayload::BeamSave))),
        _ if m == LOOK_SAVE => MasterUI(Set(BeamStoreState(BeamStoreStatePayload::LookSave))),
        _ if m == BEAM_DELETE => MasterUI(Set(BeamStoreState(BeamStoreStatePayload::Delete))),
        _ if m == LOOK_EDIT => MasterUI(Set(BeamStoreState(BeamStoreStatePayload::LookEdit))),
        _ => return None,
    })
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_master_ui_control(sc: StateChange, manager: &mut impl MidiOutput) {
    use StateChange::*;

    let mut send_main = |event| {
        manager.send(&Device::TouchOsc, event);
        manager.send(&Device::AkaiApc40, event);
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
                CHANNEL_SELECT_BUTTONS.all_off(|event| manager.send(&Device::AkaiApc20, event));
            } else {
                CHANNEL_SELECT_BUTTONS.all_off(send_main);
                CHANNEL_SELECT_BUTTONS.select(note_on(midi_channel, CHANNEL_SELECT), |event| {
                    manager.send(&Device::AkaiApc20, event)
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
                manager.send(&Device::AkaiApc20, e);
            }
        }
        BeamStoreState(state) => {
            let send_all = |event| {
                manager.send(&Device::TouchOsc, event);
                manager.send(&Device::AkaiApc40, event);
                manager.send(&Device::AkaiApc20, event);
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
