use super::{
    add_control, bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi,
    ControlMap,
};
use crate::{
    device::Device,
    midi::Event,
    midi::{EventType, Manager, Mapping},
    show::ControlMessage::Tunnel,
    tunnel::ControlMessage,
    tunnel::StateChange,
};

// Knobs
const THICKNESS: u8 = 21;
const SIZE: u8 = 22;
const COL_CENTER: u8 = 16;
const COL_WIDTH: u8 = 17;
const COL_SPREAD: u8 = 18;
const COL_SAT: u8 = 19;
const ASPECT_RATIO: u8 = 23;
const ROT_SPEED: u8 = 52;
const MARQUEE_SPEED: u8 = 20;
const BLACKING: u8 = 54;
const SEGMENTS: u8 = 53;

pub fn map_tunnel_controls(device: Device, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;
    {
        // inner lexical scope for temporary borrow of map in this closure
        let mut cc = |control, creator| {
            add_control(
                map,
                device,
                Mapping {
                    event_type: EventType::ControlChange,
                    channel: 0,
                    control: control,
                },
                creator,
            )
        };
        // unipolar knobs
        cc(THICKNESS, |v| Tunnel(Set(Thickness(unipolar_from_midi(v)))));
        cc(SIZE, |v| Tunnel(Set(Size(unipolar_from_midi(v)))));
        cc(COL_CENTER, |v| {
            Tunnel(Set(ColorCenter(unipolar_from_midi(v))))
        });
        cc(COL_WIDTH, |v| {
            Tunnel(Set(ColorWidth(unipolar_from_midi(v))))
        });
        cc(COL_SPREAD, |v| {
            Tunnel(Set(ColorSpread(unipolar_from_midi(v))))
        });
        cc(COL_SAT, |v| {
            Tunnel(Set(ColorSaturation(unipolar_from_midi(v))))
        });
        cc(ASPECT_RATIO, |v| {
            Tunnel(Set(AspectRatio(unipolar_from_midi(v))))
        });
        // bipolar knobs
        cc(ROT_SPEED, |v| {
            Tunnel(Set(RotationSpeed(bipolar_from_midi(v))))
        });
        cc(MARQUEE_SPEED, |v| {
            Tunnel(Set(MarqueeSpeed(bipolar_from_midi(v))))
        });
        cc(BLACKING, |v| Tunnel(Set(Blacking(bipolar_from_midi(v)))));
        // FIXME segments tied to midi value
        cc(SEGMENTS, |v| Tunnel(Set(Segments(v + 1))));
    }
    {
        // inner lexical scope for temporary borrow of map in this closure
        let mut note_on = |control, creator| {
            add_control(
                map,
                device,
                Mapping {
                    event_type: EventType::NoteOn,
                    channel: 0,
                    control: control,
                },
                creator,
            )
        };

        note_on(0x60, |_| Tunnel(NudgeRight));
        note_on(0x61, |_| Tunnel(NudgeLeft));
        note_on(0x5F, |_| Tunnel(NudgeUp));
        note_on(0x5E, |_| Tunnel(NudgeDown));
        note_on(0x62, |_| Tunnel(ResetPosition));
        note_on(120, |_| Tunnel(ResetRotation));
        note_on(121, |_| Tunnel(ResetMarquee));
    }
}

/// Emit midi messages to update UIs given the provided tunnel state change.
pub fn update_tunnel_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let event = match sc {
        Thickness(v) => control_event(THICKNESS, unipolar_to_midi(v)),
        Size(v) => control_event(SIZE, unipolar_to_midi(v)),
        AspectRatio(v) => control_event(ASPECT_RATIO, unipolar_to_midi(v)),
        ColorCenter(v) => control_event(COL_CENTER, unipolar_to_midi(v)),
        ColorWidth(v) => control_event(COL_WIDTH, unipolar_to_midi(v)),
        ColorSpread(v) => control_event(COL_SPREAD, unipolar_to_midi(v)),
        ColorSaturation(v) => control_event(COL_SAT, unipolar_to_midi(v)),
        Segments(v) => control_event(SEGMENTS, v - 1),
        Blacking(v) => control_event(BLACKING, bipolar_to_midi(v)),
        MarqueeSpeed(v) => control_event(MARQUEE_SPEED, bipolar_to_midi(v)),
        RotationSpeed(v) => control_event(ROT_SPEED, bipolar_to_midi(v)),
    };
    manager.send(Device::AkaiApc40, event);
    manager.send(Device::TouchOsc, event);
}

fn control_event(control: u8, value: u8) -> Event {
    Event {
        mapping: Mapping {
            event_type: EventType::ControlChange,
            channel: 0,
            control,
        },
        value,
    }
}
