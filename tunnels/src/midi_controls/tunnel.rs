use super::{bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap};
use crate::{
    device::Device,
    midi::{cc, cc_ch0, event, note_on_ch0, Manager, Mapping},
    show::ControlMessage::Tunnel,
    tunnel::ControlMessage,
    tunnel::StateChange,
};
use tunnels_lib::number::BipolarFloat;

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
const NUDGE_RIGHT: Mapping = note_on_ch0(0x60);
const NUDGE_LEFT: Mapping = note_on_ch0(0x61);
const NUDGE_UP: Mapping = note_on_ch0(0x5F);
const NUDGE_DOWN: Mapping = note_on_ch0(0x5E);
const RESET_POSITION: Mapping = note_on_ch0(0x62);
const RESET_ROTATION: Mapping = note_on_ch0(120);
const RESET_MARQUEE: Mapping = note_on_ch0(121);

// TouchOSC XY position pad.
const POSITION_X: Mapping = cc(8, 1);
const POSITION_Y: Mapping = cc(8, 0);

pub fn map_tunnel_controls(device: Device, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;
    let mut add = |mapping, creator| map.add(device, mapping, creator);

    // unipolar knobs
    add(
        THICKNESS,
        Box::new(|v| Tunnel(Set(Thickness(unipolar_from_midi(v))))),
    );
    add(SIZE, Box::new(|v| Tunnel(Set(Size(unipolar_from_midi(v))))));
    add(
        COL_CENTER,
        Box::new(|v| Tunnel(Set(ColorCenter(unipolar_from_midi(v))))),
    );
    add(
        COL_WIDTH,
        Box::new(|v| Tunnel(Set(ColorWidth(unipolar_from_midi(v))))),
    );
    add(
        COL_SPREAD,
        Box::new(|v| Tunnel(Set(ColorSpread(unipolar_from_midi(v))))),
    );
    add(
        COL_SAT,
        Box::new(|v| Tunnel(Set(ColorSaturation(unipolar_from_midi(v))))),
    );
    add(
        ASPECT_RATIO,
        Box::new(|v| Tunnel(Set(AspectRatio(unipolar_from_midi(v))))),
    );
    // bipolar knobs
    add(
        ROT_SPEED,
        Box::new(|v| Tunnel(Set(RotationSpeed(bipolar_from_midi(v))))),
    );
    add(
        MARQUEE_SPEED,
        Box::new(|v| Tunnel(Set(MarqueeSpeed(bipolar_from_midi(v))))),
    );
    add(
        BLACKING,
        Box::new(|v| Tunnel(Set(Blacking(bipolar_from_midi(v))))),
    );
    // FIXME segments tied to midi value
    add(SEGMENTS, Box::new(|v| Tunnel(Set(Segments(v + 1)))));

    add(NUDGE_RIGHT, Box::new(|_| Tunnel(NudgeRight)));
    add(NUDGE_LEFT, Box::new(|_| Tunnel(NudgeLeft)));
    add(NUDGE_UP, Box::new(|_| Tunnel(NudgeUp)));
    add(NUDGE_DOWN, Box::new(|_| Tunnel(NudgeDown)));
    add(RESET_POSITION, Box::new(|_| Tunnel(ResetPosition)));
    add(RESET_ROTATION, Box::new(|_| Tunnel(ResetRotation)));
    add(RESET_MARQUEE, Box::new(|_| Tunnel(ResetMarquee)));
    add(
        POSITION_X,
        Box::new(|v| Tunnel(Set(PositionX(bipolar_from_midi(v).val())))),
    );
    add(
        POSITION_Y,
        Box::new(|v| Tunnel(Set(PositionY(bipolar_from_midi(v).val())))),
    );
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
        // Clamp outgoing tunnel position messages to regular midi range.
        PositionX(v) => event(POSITION_X, bipolar_to_midi(BipolarFloat::new(v))),
        PositionY(v) => event(POSITION_Y, bipolar_to_midi(BipolarFloat::new(v))),
    };
    manager.send(Device::AkaiApc40, event);
    manager.send(Device::TouchOsc, event);
}
