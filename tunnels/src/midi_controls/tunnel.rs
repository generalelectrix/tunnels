use super::{bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi};
use crate::{
    midi::{cc, cc_ch0, event, note_on, note_on_ch0, Event, Mapping, MidiOutput},
    midi_controls::Device,
    midi_controls::RadioButtons,
    palette::ColorPaletteIdx,
    show::ControlMessage::Tunnel,
    tunnel::ControlMessage,
    tunnel::StateChange,
};
use lazy_static::lazy_static;
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

const SPIN_SPEED: Mapping = cc_ch0(55);
const RESET_SPIN: Mapping = note_on_ch0(122);

const RENDER_MODE_ARC: Mapping = note_on(0, 62);
const RENDER_MODE_DOT: Mapping = note_on(0, 63);
const RENDER_MODE_SAUCER: Mapping = note_on(0, 64);

// TouchOSC XY position pad.
const POSITION_X: Mapping = cc(8, 1);
const POSITION_Y: Mapping = cc(8, 0);

const PALETTE_SELECT_CONTROL_OFFSET: i32 = 59;
const N_PALETTE_SELECTS: i32 = 3;

lazy_static! {
    static ref RENDER_MODE_BUTTONS: RadioButtons = RadioButtons {
        mappings: vec!(
            RENDER_MODE_ARC,
            RENDER_MODE_DOT,
            RENDER_MODE_SAUCER,
        ),
        off: 0,
        on: 1,
    };
    static ref PALETTE_SELECT_BUTTONS: RadioButtons = RadioButtons {
        // -1 corresponds to "internal", the rest as global clock IDs.
        mappings: (-1..N_PALETTE_SELECTS)
            .map(|palette_id| note_on_ch0((palette_id + PALETTE_SELECT_CONTROL_OFFSET) as u8))
            .collect(),
        off: 0,
        on: 1,
    };
}

pub fn interpret(event: &Event) -> Option<crate::show::ControlMessage> {
    use ControlMessage::*;
    use StateChange::*;
    let v = event.value;
    Some(match event.mapping {
        THICKNESS => Tunnel(Set(Thickness(unipolar_from_midi(v)))),
        SIZE => Tunnel(Set(Size(unipolar_from_midi(v)))),
        COL_CENTER => Tunnel(Set(ColorCenter(unipolar_from_midi(v)))),
        COL_WIDTH => Tunnel(Set(ColorWidth(unipolar_from_midi(v)))),
        COL_SPREAD => Tunnel(Set(ColorSpread(unipolar_from_midi(v)))),
        COL_SAT => Tunnel(Set(ColorSaturation(unipolar_from_midi(v)))),
        ASPECT_RATIO => Tunnel(Set(AspectRatio(unipolar_from_midi(v)))),
        ROT_SPEED => Tunnel(Set(RotationSpeed(bipolar_from_midi(v)))),
        MARQUEE_SPEED => Tunnel(Set(MarqueeSpeed(bipolar_from_midi(v)))),
        BLACKING => Tunnel(Set(Blacking(bipolar_from_midi(v)))),
        SEGMENTS => Tunnel(Set(Segments(v + 1))),
        NUDGE_RIGHT => Tunnel(NudgeRight),
        NUDGE_LEFT => Tunnel(NudgeLeft),
        NUDGE_UP => Tunnel(NudgeUp),
        NUDGE_DOWN => Tunnel(NudgeDown),
        RESET_POSITION => Tunnel(ResetPosition),
        RESET_ROTATION => Tunnel(ResetRotation),
        RESET_MARQUEE => Tunnel(ResetMarquee),
        SPIN_SPEED => Tunnel(Set(SpinSpeed(bipolar_from_midi(v)))),
        RESET_SPIN => Tunnel(ResetSpin),
        POSITION_X => Tunnel(Set(PositionX(bipolar_from_midi(v).val()))),
        POSITION_Y => Tunnel(Set(PositionY(bipolar_from_midi(v).val()))),
        RENDER_MODE_ARC => Tunnel(Set(RenderMode(tunnels_lib::RenderMode::Arc))),
        RENDER_MODE_DOT => Tunnel(Set(RenderMode(tunnels_lib::RenderMode::Dot))),
        RENDER_MODE_SAUCER => Tunnel(Set(RenderMode(tunnels_lib::RenderMode::Saucer))),
        m if m.event_type == crate::midi::EventType::NoteOn
            && m.channel == 0
            && m.control >= (PALETTE_SELECT_CONTROL_OFFSET - 1) as u8
            && m.control < (PALETTE_SELECT_CONTROL_OFFSET + N_PALETTE_SELECTS) as u8 =>
        {
            let palette_id = m.control as i32 - PALETTE_SELECT_CONTROL_OFFSET;
            if palette_id < 0 {
                Tunnel(Set(PaletteSelection(None)))
            } else {
                Tunnel(Set(PaletteSelection(Some(ColorPaletteIdx(
                    palette_id as usize,
                )))))
            }
        }
        _ => return None,
    })
}

/// Emit midi messages to update UIs given the provided tunnel state change.
pub fn update_tunnel_control(sc: StateChange, manager: &mut impl MidiOutput) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(&Device::AkaiApc40, event);
        manager.send(&Device::TouchOsc, event);
    };

    match sc {
        Thickness(v) => send(event(THICKNESS, unipolar_to_midi(v))),
        Size(v) => send(event(SIZE, unipolar_to_midi(v))),
        AspectRatio(v) => send(event(ASPECT_RATIO, unipolar_to_midi(v))),
        ColorCenter(v) => send(event(COL_CENTER, unipolar_to_midi(v))),
        ColorWidth(v) => send(event(COL_WIDTH, unipolar_to_midi(v))),
        ColorSpread(v) => send(event(COL_SPREAD, unipolar_to_midi(v))),
        ColorSaturation(v) => send(event(COL_SAT, unipolar_to_midi(v))),
        PaletteSelection(v) => {
            let index = match v {
                Some(source) => source.0 as i32,
                None => -1,
            };
            PALETTE_SELECT_BUTTONS.select(
                note_on_ch0((index + PALETTE_SELECT_CONTROL_OFFSET) as u8),
                send,
            );
        }
        Segments(v) => send(event(SEGMENTS, v - 1)),
        Blacking(v) => send(event(BLACKING, bipolar_to_midi(v))),
        MarqueeSpeed(v) => send(event(MARQUEE_SPEED, bipolar_to_midi(v))),
        RotationSpeed(v) => send(event(ROT_SPEED, bipolar_to_midi(v))),
        // Clamp outgoing tunnel position messages to regular midi range.
        PositionX(v) => send(event(POSITION_X, bipolar_to_midi(BipolarFloat::new(v)))),
        PositionY(v) => send(event(POSITION_Y, bipolar_to_midi(BipolarFloat::new(v)))),
        SpinSpeed(v) => send(event(SPIN_SPEED, bipolar_to_midi(v))),
        RenderMode(v) => {
            use tunnels_lib::RenderMode::*;
            RENDER_MODE_BUTTONS.select(
                match v {
                    Arc => RENDER_MODE_ARC,
                    Dot => RENDER_MODE_DOT,
                    Saucer => RENDER_MODE_SAUCER,
                },
                send,
            );
        }
    };
}
