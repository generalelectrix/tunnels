use crate::{
    animation::ControlMessage,
    animation::StateChange,
    animation::Target as AnimationTarget,
    animation::Waveform as WaveformType,
    clock_bank::ClockIdx,
    device::Device,
    midi::{cc_ch0, event, note_on_ch0, note_on_ch1, Manager, Mapping},
    show::ControlMessage::Animation,
};
use lazy_static::lazy_static;

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, ControlMap,
    RadioButtons,
};

// knobs
const SPEED: Mapping = cc_ch0(48);
const WEIGHT: Mapping = cc_ch0(49);
const DUTY_CYCLE: Mapping = cc_ch0(50);
const SMOOTHING: Mapping = cc_ch0(51);

// waveform type buttons
const SINE: Mapping = note_on_ch0(24);
const TRIANGLE: Mapping = note_on_ch0(25);
const SQUARE: Mapping = note_on_ch0(26);
const SAWTOOTH: Mapping = note_on_ch0(27);

// target buttons
const ROTATION: Mapping = note_on_ch0(35);
const THICKNESS: Mapping = note_on_ch0(36);
const SIZE: Mapping = note_on_ch0(37);
const ASPECT_RATIO: Mapping = note_on_ch0(38);
const COLOR: Mapping = note_on_ch0(39);
const COLOR_SPREAD: Mapping = note_on_ch0(40);
const COLOR_PERIODICITY: Mapping = note_on_ch0(41);
const COLOR_SATURATION: Mapping = note_on_ch0(42);
const MARQUEE: Mapping = note_on_ch0(43);
const SEGMENTS: Mapping = note_on_ch0(44);
const BLACKING: Mapping = note_on_ch0(45);
const POSITIONX: Mapping = note_on_ch0(46);
const POSITIONY: Mapping = note_on_ch0(47);

// These buttons are on channel 1 instead of 0 as we ran out of space on channel 1.
const PULSE: Mapping = note_on_ch1(0);
const INVERT: Mapping = note_on_ch1(1);

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
            ROTATION,
            THICKNESS,
            SIZE,
            ASPECT_RATIO,
            COLOR,
            COLOR_SPREAD,
            COLOR_PERIODICITY,
            COLOR_SATURATION,
            MARQUEE,
            SEGMENTS,
            BLACKING,
            POSITIONX,
            POSITIONY,
        ),
        off: 0, on: 1
    };
    static ref CLOCK_SELECT_BUTTONS: RadioButtons = RadioButtons {
        // -1 corresponds to "internal", the rest as global clock IDs.
        mappings: (-1..8)
            .map(|clock_id| note_on_ch1((clock_id + CLOCK_SELECT_CONTROL_OFFSET) as u8))
            .collect(),
        off: 0,
        on: 1,
    };
}

pub fn map_animation_controls(device: Device, map: &mut ControlMap) {
    use AnimationTarget::*;
    use ControlMessage::*;
    use StateChange::*;
    use WaveformType::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    add(
        SPEED,
        Box::new(|v| Animation(Set(Speed(bipolar_from_midi(v))))),
    );
    add(
        WEIGHT,
        Box::new(|v| Animation(Set(Weight(unipolar_from_midi(v))))),
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
    add(ROTATION, Box::new(|_| Animation(Set(Target(Rotation)))));
    add(THICKNESS, Box::new(|_| Animation(Set(Target(Thickness)))));
    add(SIZE, Box::new(|_| Animation(Set(Target(Size)))));
    add(
        ASPECT_RATIO,
        Box::new(|_| Animation(Set(Target(AspectRatio)))),
    );
    add(COLOR, Box::new(|_| Animation(Set(Target(Color)))));
    add(
        COLOR_SPREAD,
        Box::new(|_| Animation(Set(Target(ColorSpread)))),
    );
    add(
        COLOR_PERIODICITY,
        Box::new(|_| Animation(Set(Target(ColorPeriodicity)))),
    );
    add(
        COLOR_SATURATION,
        Box::new(|_| Animation(Set(Target(ColorSaturation)))),
    );
    add(
        MARQUEE,
        Box::new(|_| Animation(Set(Target(MarqueeRotation)))),
    );
    add(SEGMENTS, Box::new(|_| Animation(Set(Target(Segments)))));
    add(BLACKING, Box::new(|_| Animation(Set(Target(Blacking)))));
    add(POSITIONX, Box::new(|_| Animation(Set(Target(PositionX)))));
    add(POSITIONY, Box::new(|_| Animation(Set(Target(PositionY)))));

    // pulse/invert
    add(PULSE, Box::new(|_| Animation(TogglePulse)));
    add(INVERT, Box::new(|_| Animation(ToggleInvert)));

    // clock select
    add(
        note_on_ch1((CLOCK_SELECT_CONTROL_OFFSET - 1) as u8),
        Box::new(|_| Animation(Set(ClockSource(None)))),
    );
    for clock_num in 0..8 {
        add(
            note_on_ch1((CLOCK_SELECT_CONTROL_OFFSET + clock_num) as u8),
            Box::new(move |_| Animation(Set(ClockSource(Some(ClockIdx(clock_num as usize)))))),
        );
    }
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
        Weight(v) => send(event(WEIGHT, unipolar_to_midi(v))),
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
                    Rotation => ROTATION,
                    Thickness => THICKNESS,
                    Size => SIZE,
                    AspectRatio => ASPECT_RATIO,
                    Color => COLOR,
                    ColorSpread => COLOR_SPREAD,
                    ColorPeriodicity => COLOR_PERIODICITY,
                    ColorSaturation => COLOR_SATURATION,
                    MarqueeRotation => MARQUEE,
                    Segments => SEGMENTS,
                    Blacking => BLACKING,
                    PositionX => POSITIONX,
                    PositionY => POSITIONY,
                },
                send,
            );
        }
        Invert(v) => send(event(INVERT, v as u8)),
        Pulse(v) => send(event(PULSE, v as u8)),
        ClockSource(v) => {
            let index = match v {
                Some(source) => (source.0 as i32),
                None => -1,
            };
            CLOCK_SELECT_BUTTONS.select(
                note_on_ch1((index + CLOCK_SELECT_CONTROL_OFFSET) as u8),
                send,
            );
        }
    }
}
