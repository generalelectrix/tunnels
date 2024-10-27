use crate::{
    animation_target::AnimationTarget as AnimationTargetState,
    midi::{note_on_ch0, Manager, Mapping},
    midi_controls::Device,
    show::ControlMessage::AnimationTarget as Animation,
};
use lazy_static::lazy_static;

use super::{ControlMap, RadioButtons};

const TARGET_ROTATION: Mapping = note_on_ch0(35);
const TARGET_THICKNESS: Mapping = note_on_ch0(36);
const TARGET_SIZE: Mapping = note_on_ch0(37);
const TARGET_ASPECT_RATIO: Mapping = note_on_ch0(38);
const TARGET_COLOR: Mapping = note_on_ch0(39);
const TARGET_COLOR_SPREAD: Mapping = note_on_ch0(40);
const TARGET_COLOR_PERIODICITY: Mapping = note_on_ch0(41);
const TARGET_COLOR_SATURATION: Mapping = note_on_ch0(42);
const TARGET_MARQUEE: Mapping = note_on_ch0(43);
const TARGET_POSITIONX: Mapping = note_on_ch0(44);
const TARGET_POSITIONY: Mapping = note_on_ch0(45);

lazy_static! {
    static ref TARGET_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: vec!(
            TARGET_ROTATION,
            TARGET_THICKNESS,
            TARGET_SIZE,
            TARGET_ASPECT_RATIO,
            TARGET_COLOR,
            TARGET_COLOR_SPREAD,
            TARGET_COLOR_PERIODICITY,
            TARGET_COLOR_SATURATION,
            TARGET_MARQUEE,
            TARGET_POSITIONX,
            TARGET_POSITIONY,
        ),
        off: 0,
        on: 1
    };
}

pub fn map_animation_target_controls(device: Device, map: &mut ControlMap) {
    let mut add = |mapping, creator| map.add(device, mapping, creator);

    // target select
    add(
        TARGET_ROTATION,
        Box::new(|_| Animation(AnimationTargetState::Rotation)),
    );
    add(
        TARGET_THICKNESS,
        Box::new(|_| Animation(AnimationTargetState::Thickness)),
    );
    add(
        TARGET_SIZE,
        Box::new(|_| Animation(AnimationTargetState::Size)),
    );
    add(
        TARGET_ASPECT_RATIO,
        Box::new(|_| Animation(AnimationTargetState::AspectRatio)),
    );
    add(
        TARGET_COLOR,
        Box::new(|_| Animation(AnimationTargetState::Color)),
    );
    add(
        TARGET_COLOR_SPREAD,
        Box::new(|_| Animation(AnimationTargetState::ColorSpread)),
    );
    add(
        TARGET_COLOR_PERIODICITY,
        Box::new(|_| Animation(AnimationTargetState::ColorPeriodicity)),
    );
    add(
        TARGET_COLOR_SATURATION,
        Box::new(|_| Animation(AnimationTargetState::ColorSaturation)),
    );
    add(
        TARGET_MARQUEE,
        Box::new(|_| Animation(AnimationTargetState::MarqueeRotation)),
    );
    add(
        TARGET_POSITIONX,
        Box::new(|_| Animation(AnimationTargetState::PositionX)),
    );
    add(
        TARGET_POSITIONY,
        Box::new(|_| Animation(AnimationTargetState::PositionY)),
    );
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_animation_target_control(sc: AnimationTargetState, manager: &mut Manager<Device>) {
    let send = |event| {
        manager.send(Device::TouchOsc, event);
    };

    use AnimationTargetState::*;
    TARGET_SELECT_BUTTONS.select(
        match sc {
            Rotation => TARGET_ROTATION,
            Thickness => TARGET_THICKNESS,
            Size => TARGET_SIZE,
            AspectRatio => TARGET_ASPECT_RATIO,
            Color => TARGET_COLOR,
            ColorSpread => TARGET_COLOR_SPREAD,
            ColorPeriodicity => TARGET_COLOR_PERIODICITY,
            ColorSaturation => TARGET_COLOR_SATURATION,
            MarqueeRotation => TARGET_MARQUEE,
            PositionX => TARGET_POSITIONX,
            PositionY => TARGET_POSITIONY,
        },
        send,
    );
}
