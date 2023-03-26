use crate::{
    animation::ControlMessage,
    animation::StateChange,
    animation::Target as AnimationTarget,
    animation::Waveform as WaveformType,
    clock_bank::{ClockIdxExt, N_CLOCKS},
    midi::{cc_ch0, event, note_on_ch0, note_on_ch1, Manager, Mapping},
    midi_controls::Device,
    show::ControlMessage::Animation,
};
use lazy_static::lazy_static;

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap,
    RadioButtons,
};

// knobs
const SPEED: Mapping = cc_ch0(48);
const SIZE: Mapping = cc_ch0(49);
const DUTY_CYCLE: Mapping = cc_ch0(50);
const SMOOTHING: Mapping = cc_ch0(51);

// waveform type buttons
const SINE: Mapping = note_on_ch0(24);
const TRIANGLE: Mapping = note_on_ch0(25);
const SQUARE: Mapping = note_on_ch0(26);
const SAWTOOTH: Mapping = note_on_ch0(27);

// target buttons
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

// These buttons are on channel 1 instead of 0 as we ran out of space on channel 1.
const PULSE: Mapping = note_on_ch1(0);
const INVERT: Mapping = note_on_ch1(1);
const USE_AUDIO_SIZE: Mapping = note_on_ch1(2);
const USE_AUDIO_SPEED: Mapping = note_on_ch1(3);
const STANDING: Mapping = note_on_ch1(7);

const CLOCK_SELECT_CONTROL_OFFSET: i32 = 112;

lazy_static! {
    static ref WAVEFORM_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: vec!(SINE, TRIANGLE, SQUARE, SAWTOOTH), off: 0, on: 1,
    };
    static ref N_PERIODS_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: (0..15).map(note_on_ch0).collect(), off: 0, on: 1,
    };
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
        off: 0, on: 1
    };
    static ref CLOCK_SELECT_BUTTONS: RadioButtons = RadioButtons {
        // -1 corresponds to "internal", the rest as global clock IDs.
        mappings: (-1..N_CLOCKS as i32)
            .map(|clock_id| note_on_ch0((clock_id + CLOCK_SELECT_CONTROL_OFFSET) as u8))
            .collect(),
        off: 0,
        on: 1,
    };
}

pub fn map_animation_controls(device: Device, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;
    use WaveformType::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    add(
        SPEED,
        Box::new(|v| Animation(Set(Speed(bipolar_from_midi(v))))),
    );
    add(
        SIZE,
        Box::new(|v| Animation(Set(Size(unipolar_from_midi(v))))),
    );
    add(
        DUTY_CYCLE,
        Box::new(|v| Animation(Set(DutyCycle(unipolar_from_midi(v))))),
    );
    add(
        SMOOTHING,
        Box::new(|v| Animation(Set(Smoothing(unipolar_from_midi(v))))),
    );

    // waveform select
    add(SINE, Box::new(|_| Animation(Set(Waveform(Sine)))));
    add(TRIANGLE, Box::new(|_| Animation(Set(Waveform(Triangle)))));
    add(SQUARE, Box::new(|_| Animation(Set(Waveform(Square)))));
    add(SAWTOOTH, Box::new(|_| Animation(Set(Waveform(Sawtooth)))));

    // n periods select
    for n_periods in 0..16 {
        add(
            note_on_ch0(n_periods as u8),
            Box::new(move |_| Animation(Set(NPeriods(n_periods)))),
        );
    }

    // target select
    add(
        TARGET_ROTATION,
        Box::new(|_| Animation(Set(Target(AnimationTarget::Rotation)))),
    );
    add(
        TARGET_THICKNESS,
        Box::new(|_| Animation(Set(Target(AnimationTarget::Thickness)))),
    );
    add(
        TARGET_SIZE,
        Box::new(|_| Animation(Set(Target(AnimationTarget::Size)))),
    );
    add(
        TARGET_ASPECT_RATIO,
        Box::new(|_| Animation(Set(Target(AnimationTarget::AspectRatio)))),
    );
    add(
        TARGET_COLOR,
        Box::new(|_| Animation(Set(Target(AnimationTarget::Color)))),
    );
    add(
        TARGET_COLOR_SPREAD,
        Box::new(|_| Animation(Set(Target(AnimationTarget::ColorSpread)))),
    );
    add(
        TARGET_COLOR_PERIODICITY,
        Box::new(|_| Animation(Set(Target(AnimationTarget::ColorPeriodicity)))),
    );
    add(
        TARGET_COLOR_SATURATION,
        Box::new(|_| Animation(Set(Target(AnimationTarget::ColorSaturation)))),
    );
    add(
        TARGET_MARQUEE,
        Box::new(|_| Animation(Set(Target(AnimationTarget::MarqueeRotation)))),
    );
    add(
        TARGET_POSITIONX,
        Box::new(|_| Animation(Set(Target(AnimationTarget::PositionX)))),
    );
    add(
        TARGET_POSITIONY,
        Box::new(|_| Animation(Set(Target(AnimationTarget::PositionY)))),
    );

    // pulse/invert/standing wave
    add(PULSE, Box::new(|_| Animation(TogglePulse)));
    add(INVERT, Box::new(|_| Animation(ToggleInvert)));
    add(STANDING, Box::new(|_| Animation(ToggleStanding)));

    // clock select
    add(
        note_on_ch0((CLOCK_SELECT_CONTROL_OFFSET - 1) as u8),
        Box::new(|_| Animation(Set(ClockSource(None)))),
    );
    for clock_num in 0..N_CLOCKS as i32 {
        add(
            note_on_ch0((CLOCK_SELECT_CONTROL_OFFSET + clock_num) as u8),
            Box::new(move |_| Animation(SetClockSource(Some(ClockIdxExt(clock_num as usize))))),
        );
    }

    add(USE_AUDIO_SIZE, Box::new(|_| Animation(ToggleUseAudioSize)));
    add(
        USE_AUDIO_SPEED,
        Box::new(|_| Animation(ToggleUseAudioSpeed)),
    );
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_animation_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(Device::AkaiApc40, event);
        manager.send(Device::TouchOsc, event);
    };

    match sc {
        Speed(v) => send(event(SPEED, bipolar_to_midi(v))),
        Size(v) => send(event(SIZE, unipolar_to_midi(v))),
        DutyCycle(v) => send(event(DUTY_CYCLE, unipolar_to_midi(v))),
        Smoothing(v) => send(event(SMOOTHING, unipolar_to_midi(v))),
        Waveform(v) => {
            use WaveformType::*;
            WAVEFORM_SELECT_BUTTONS.select(
                match v {
                    Sine => SINE,
                    Triangle => TRIANGLE,
                    Square => SQUARE,
                    Sawtooth => SAWTOOTH,
                },
                send,
            );
        }
        NPeriods(v) => N_PERIODS_SELECT_BUTTONS.select(note_on_ch0(v as u8), send),
        Target(v) => {
            use AnimationTarget::*;
            TARGET_SELECT_BUTTONS.select(
                match v {
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
        Invert(v) => send(event(INVERT, v as u8)),
        Standing(v) => send(event(STANDING, v as u8)),
        Pulse(v) => send(event(PULSE, v as u8)),
        ClockSource(v) => {
            let index = match v {
                Some(source) => usize::from(source) as i32,
                None => -1,
            };
            CLOCK_SELECT_BUTTONS.select(
                note_on_ch0((index as i32 + CLOCK_SELECT_CONTROL_OFFSET) as u8),
                send,
            );
        }
        UseAudioSize(v) => send(event(USE_AUDIO_SIZE, v as u8)),
        UseAudioSpeed(v) => send(event(USE_AUDIO_SPEED, v as u8)),
    }
}
