use crate::{
    animation_target::AnimationTarget as AnimationTargetState,
    midi::{note_on_ch0, Event, MidiOutput, Mapping},
    midi_controls::Device,
    show::ControlMessage::AnimationTarget as Animation,
};
use lazy_static::lazy_static;

use super::RadioButtons;

const TARGET_ROTATION: Mapping = note_on_ch0(35);
const TARGET_THICKNESS: Mapping = note_on_ch0(36);
const TARGET_SIZE: Mapping = note_on_ch0(37);
const TARGET_ASPECT_RATIO: Mapping = note_on_ch0(38);
const TARGET_MARQUEE: Mapping = note_on_ch0(39);
const TARGET_SPIN: Mapping = note_on_ch0(40);
const TARGET_COLOR: Mapping = note_on_ch0(41);
const TARGET_COLOR_SATURATION: Mapping = note_on_ch0(42);

const TARGET_POSITIONX: Mapping = note_on_ch0(43);
const TARGET_POSITIONY: Mapping = note_on_ch0(44);

lazy_static! {
    static ref TARGET_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: vec!(
            TARGET_ROTATION,
            TARGET_THICKNESS,
            TARGET_SIZE,
            TARGET_ASPECT_RATIO,
            TARGET_MARQUEE,
            TARGET_SPIN,
            TARGET_COLOR,
            TARGET_COLOR_SATURATION,
            TARGET_POSITIONX,
            TARGET_POSITIONY,
        ),
        off: 0,
        on: 1
    };
}

pub fn interpret(event: &Event) -> Option<crate::show::ControlMessage> {
    Some(match event.mapping {
        TARGET_ROTATION => Animation(AnimationTargetState::Rotation),
        TARGET_THICKNESS => Animation(AnimationTargetState::Thickness),
        TARGET_SIZE => Animation(AnimationTargetState::Size),
        TARGET_ASPECT_RATIO => Animation(AnimationTargetState::AspectRatio),
        TARGET_MARQUEE => Animation(AnimationTargetState::MarqueeRotation),
        TARGET_SPIN => Animation(AnimationTargetState::Spin),
        TARGET_COLOR => Animation(AnimationTargetState::Color),
        TARGET_COLOR_SATURATION => Animation(AnimationTargetState::ColorSaturation),
        TARGET_POSITIONX => Animation(AnimationTargetState::PositionX),
        TARGET_POSITIONY => Animation(AnimationTargetState::PositionY),
        _ => return None,
    })
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_animation_target_control(sc: AnimationTargetState, manager: &mut impl MidiOutput) {
    let send = |event| {
        manager.send(&Device::TouchOsc, event);
    };

    use AnimationTargetState::*;
    TARGET_SELECT_BUTTONS.select(
        match sc {
            Rotation => TARGET_ROTATION,
            Thickness => TARGET_THICKNESS,
            Size => TARGET_SIZE,
            AspectRatio => TARGET_ASPECT_RATIO,
            Color => TARGET_COLOR,
            ColorSaturation => TARGET_COLOR_SATURATION,
            MarqueeRotation => TARGET_MARQUEE,
            PositionX => TARGET_POSITIONX,
            PositionY => TARGET_POSITIONY,
            Spin => TARGET_SPIN,
            ColorSpread => return,
        },
        send,
    );
}
