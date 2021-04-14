use crate::{
    clock::ClockIdx,
    clock::ControlMessage,
    clock::StateChange,
    clock::N_CLOCKS,
    device::Device,
    midi::{cc, event, note_off, note_on, Manager, Mapping},
    show::ControlMessage as ShowControlMessage,
};

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap,
    RadioButtons,
};

pub fn map_clock_controls(device: Device, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    // TODO: revamp clock interface using new hardware
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_clock_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(Device::TouchOsc, event);
    };

    // TODO: revamp clock interface using new hardware
}
