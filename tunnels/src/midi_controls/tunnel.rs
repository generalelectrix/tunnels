use super::{bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap};
use crate::{
    device::Device,
    midi::{cc_ch0, event, note_ch0, Manager, Mapping},
    show::ControlMessage::Tunnel,
    tunnel::ControlMessage,
    tunnel::StateChange,
};

// Knobs
const THICKNESS: Mapping = cc_ch0(21);
const SIZE: Mapping = cc_ch0(22);
const COL_CENTER: Mapping = cc_ch0(16);
const COL_WIDTH: Mapping = cc_ch0(17);
const COL_SPREAD: Mapping = cc_ch0(18);
const COL_SAT: Mapping = cc_ch0(19);
const ASPECT_RATIO: Mapping = cc_ch0(23);
const ROT_SPEED: Mapping = cc_ch0(52);
const MARQUEE_SPEED: Mapping = cc_ch0(20);
const BLACKING: Mapping = cc_ch0(54);
const SEGMENTS: Mapping = cc_ch0(53);

// Buttons
const NUDGE_RIGHT: Mapping = note_ch0(0x60);
const NUDGE_LEFT: Mapping = note_ch0(0x61);
const NUDGE_UP: Mapping = note_ch0(0x5F);
const NUDGE_DOWN: Mapping = note_ch0(0x5E);
const RESET_POSITION: Mapping = note_ch0(0x62);
const RESET_ROTATION: Mapping = note_ch0(120);
const RESET_MARQUEE: Mapping = note_ch0(121);

pub fn map_tunnel_controls(device: Device, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;
    let mut add = |mapping, creator| map.add(device, mapping, creator);

    // unipolar knobs
    add(THICKNESS, |v| Tunnel(Set(Thickness(unipolar_from_midi(v)))));
    add(SIZE, |v| Tunnel(Set(Size(unipolar_from_midi(v)))));
    add(COL_CENTER, |v| {
        Tunnel(Set(ColorCenter(unipolar_from_midi(v))))
    });
    add(COL_WIDTH, |v| {
        Tunnel(Set(ColorWidth(unipolar_from_midi(v))))
    });
    add(COL_SPREAD, |v| {
        Tunnel(Set(ColorSpread(unipolar_from_midi(v))))
    });
    add(COL_SAT, |v| {
        Tunnel(Set(ColorSaturation(unipolar_from_midi(v))))
    });
    add(ASPECT_RATIO, |v| {
        Tunnel(Set(AspectRatio(unipolar_from_midi(v))))
    });
    // bipolar knobs
    add(ROT_SPEED, |v| {
        Tunnel(Set(RotationSpeed(bipolar_from_midi(v))))
    });
    add(MARQUEE_SPEED, |v| {
        Tunnel(Set(MarqueeSpeed(bipolar_from_midi(v))))
    });
    add(BLACKING, |v| Tunnel(Set(Blacking(bipolar_from_midi(v)))));
    // FIXME segments tied to midi value
    add(SEGMENTS, |v| Tunnel(Set(Segments(v + 1))));

    add(NUDGE_RIGHT, |_| Tunnel(NudgeRight));
    add(NUDGE_LEFT, |_| Tunnel(NudgeLeft));
    add(NUDGE_UP, |_| Tunnel(NudgeUp));
    add(NUDGE_DOWN, |_| Tunnel(NudgeDown));
    add(RESET_POSITION, |_| Tunnel(ResetPosition));
    add(RESET_ROTATION, |_| Tunnel(ResetRotation));
    add(RESET_MARQUEE, |_| Tunnel(ResetMarquee));
}

/// Emit midi messages to update UIs given the provided tunnel state change.
pub fn update_tunnel_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let event = match sc {
        Thickness(v) => event(THICKNESS, unipolar_to_midi(v)),
        Size(v) => event(SIZE, unipolar_to_midi(v)),
        AspectRatio(v) => event(ASPECT_RATIO, unipolar_to_midi(v)),
        ColorCenter(v) => event(COL_CENTER, unipolar_to_midi(v)),
        ColorWidth(v) => event(COL_WIDTH, unipolar_to_midi(v)),
        ColorSpread(v) => event(COL_SPREAD, unipolar_to_midi(v)),
        ColorSaturation(v) => event(COL_SAT, unipolar_to_midi(v)),
        Segments(v) => event(SEGMENTS, v - 1),
        Blacking(v) => event(BLACKING, bipolar_to_midi(v)),
        MarqueeSpeed(v) => event(MARQUEE_SPEED, bipolar_to_midi(v)),
        RotationSpeed(v) => event(ROT_SPEED, bipolar_to_midi(v)),
    };
    manager.send(Device::AkaiApc40, event);
    manager.send(Device::TouchOsc, event);
}
