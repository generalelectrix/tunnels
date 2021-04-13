use std::collections::HashMap;

use crate::{
    device::Device,
    midi::{Event, EventType, Mapping},
    numbers::{BipolarFloat, UnipolarFloat},
    show::ControlMessage,
    tunnel::ControlMessage as TCM,
};

type ControlMessageCreator = fn(u8) -> ControlMessage;

type ControlMap = HashMap<(Device, Mapping), ControlMessageCreator>;
pub struct Dispatcher {
    map: ControlMap,
}

impl Dispatcher {
    /// Instantiate the master midi control dispatcher.
    pub fn new() -> Self {
        let mut map = HashMap::new();
        tunnel_controls(Device::AkaiApc40, &mut map);
        tunnel_controls(Device::TouchOsc, &mut map);
        Self { map }
    }
}

fn tunnel_controls(device: Device, map: &mut ControlMap) {
    {
        // inner lexical scope for temporary borrow of map in this closure
        let mut cc = |control, creator| {
            add_control(map, device, EventType::ControlChange, 0, control, creator)
        };
        // unipolar knobs
        cc(21, |v| {
            ControlMessage::Tunnel(TCM::Thickness(unipolar_from_midi(v)))
        });
        cc(22, |v| {
            ControlMessage::Tunnel(TCM::Size(unipolar_from_midi(v)))
        });
        cc(16, |v| {
            ControlMessage::Tunnel(TCM::ColorCenter(unipolar_from_midi(v)))
        });
        cc(17, |v| {
            ControlMessage::Tunnel(TCM::ColorWidth(unipolar_from_midi(v)))
        });
        cc(18, |v| {
            ControlMessage::Tunnel(TCM::ColorSpread(unipolar_from_midi(v)))
        });
        cc(19, |v| {
            ControlMessage::Tunnel(TCM::ColorSaturation(unipolar_from_midi(v)))
        });
        cc(23, |v| {
            ControlMessage::Tunnel(TCM::AspectRatio(unipolar_from_midi(v)))
        });
        // bipolar knobs
        cc(52, |v| {
            ControlMessage::Tunnel(TCM::RotationSpeed(bipolar_from_midi(v)))
        });
        cc(20, |v| {
            ControlMessage::Tunnel(TCM::MarqueeSpeed(bipolar_from_midi(v)))
        });
        cc(54, |v| {
            ControlMessage::Tunnel(TCM::Blacking(bipolar_from_midi(v)))
        });
        cc(53, |v| ControlMessage::Tunnel(TCM::Segments(v as u32 + 1)));
    }
    {
        // inner lexical scope for temporary borrow of map in this closure
        let mut note_on =
            |control, creator| add_control(map, device, EventType::NoteOn, 0, control, creator);

        note_on(0x60, |_| ControlMessage::Tunnel(TCM::NudgeRight));
        note_on(0x61, |_| ControlMessage::Tunnel(TCM::NudgeLeft));
        note_on(0x5F, |_| ControlMessage::Tunnel(TCM::NudgeUp));
        note_on(0x5E, |_| ControlMessage::Tunnel(TCM::NudgeDown));
        note_on(0x62, |_| ControlMessage::Tunnel(TCM::ResetPosition));
        note_on(120, |_| ControlMessage::Tunnel(TCM::ResetRotation));
        note_on(121, |_| ControlMessage::Tunnel(TCM::ResetMarquee));
    }
}

fn add_control(
    map: &mut ControlMap,
    device: Device,
    event_type: EventType,
    channel: u8,
    control: u8,
    creator: ControlMessageCreator,
) {
    let mapping = Mapping {
        event_type,
        channel,
        control,
    };
    if map.insert((device, mapping), creator).is_some() {
        panic!("duplicate control definition: {:?} {:?}", device, mapping);
    }
}

fn bipolar_from_midi(val: u8) -> BipolarFloat {
    let denom = if val > 64 { 63. } else { 64. };
    BipolarFloat((val - 64) as f64 / denom)
}

fn bipolar_to_midi(val: BipolarFloat) -> u8 {
    u16::min((((val.0 + 1.0) / 2.0) * 128.) as u16, 127) as u8
}

fn unipolar_from_midi(val: u8) -> UnipolarFloat {
    UnipolarFloat(val as f64 / 127.)
}

fn unipolar_to_midi(val: UnipolarFloat) -> u8 {
    (val.0 * 127.) as u8
}
