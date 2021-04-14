use crate::{
    clock::ClockIdx,
    device::Device,
    midi::{cc_ch0, event, note_ch0, note_ch1, Manager, Mapping},
    mixer::ControlMessage,
    mixer::StateChange,
    mixer::{ChannelControlMessage, ChannelStateChange},
    show::ControlMessage::Mixer,
};
use lazy_static::lazy_static;

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap,
    RadioButtons,
};

/// The number of mixer channels on a single mixer page.
const PAGE_SIZE: usize = 8;

pub fn map_mixer_controls(device: Device, page: usize, map: &mut ControlMap) {
    use ChannelControlMessage::*;
    use ChannelStateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    // Offset the mixer channels to correspond to this page.
    let channel_offset = page * PAGE_SIZE;
}
