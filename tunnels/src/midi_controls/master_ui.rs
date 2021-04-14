use super::{
    bipolar_from_midi, bipolar_to_midi, mixer::PAGE_SIZE, unipolar_from_midi, unipolar_to_midi,
    ControlMap, RadioButtons,
};
use crate::{
    device::Device,
    master_ui::ControlMessage,
    master_ui::StateChange,
    midi::{cc, event, note_off, note_on_ch0, Manager, Mapping},
    show::ControlMessage as ShowControlMessage,
    show::ControlMessage::MasterUI,
    tunnel::{AnimationIdx, N_ANIM},
};
use lazy_static::lazy_static;

const CHANNEL_SELECT: u8 = 0x33;
const ANIM_0_BUTTON: u8 = 0x57;

lazy_static! {
    static ref animation_select_buttons: RadioButtons = RadioButtons {
        mappings: (0..N_ANIM)
            .map(|aid| note_on_ch0(aid as u8 + ANIM_0_BUTTON))
            .collect(),
    };
}

pub fn map_master_ui_controls(device: Device, page: usize, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);
    for aid in 0..N_ANIM {
        add(
            note_on_ch0(ANIM_0_BUTTON + aid as u8),
            Box::new(move |_| MasterUI(Set(Animation(AnimationIdx(aid))))),
        );
    }
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_master_ui_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(Device::TouchOsc, event);
    };

    // TODO: revamp clock interface using new hardware
}
