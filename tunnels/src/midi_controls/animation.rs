use crate::{
    animation::ControlMessage,
    animation::StateChange,
    animation::Target as AnimationTarget,
    animation::Waveform as WaveformType,
    clock::ClockIdx,
    device::Device,
    midi::{cc_ch0, event, note_ch0, note_ch1, Manager, Mapping},
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
const SINE: Mapping = note_ch0(24);
const TRIANGLE: Mapping = note_ch0(25);
const SQUARE: Mapping = note_ch0(26);
const SAWTOOTH: Mapping = note_ch0(27);

// target buttons
const ROTATION: Mapping = note_ch0(35);
const THICKNESS: Mapping = note_ch0(36);
const SIZE: Mapping = note_ch0(37);
const ASPECT_RATIO: Mapping = note_ch0(38);
const COLOR: Mapping = note_ch0(39);
const COLOR_SPREAD: Mapping = note_ch0(40);
const COLOR_PERIODICITY: Mapping = note_ch0(41);
const COLOR_SATURATION: Mapping = note_ch0(42);
const MARQUEE: Mapping = note_ch0(43);
const SEGMENTS: Mapping = note_ch0(44);
const BLACKING: Mapping = note_ch0(45);
const POSITIONX: Mapping = note_ch0(46);
const POSITIONY: Mapping = note_ch0(47);

// These buttons are on channel 1 instead of 0 as we ran out of space on channel 1.
const PULSE: Mapping = note_ch1(0);
const INVERT: Mapping = note_ch1(1);

const CLOCK_SELECT_CONTROL_OFFSET: i32 = 112;

lazy_static! {
    static ref waveform_select_buttons: RadioButtons = RadioButtons {
        mappings: vec!(SINE, TRIANGLE, SQUARE, SAWTOOTH)
    };
    static ref n_periods_select_buttons: RadioButtons = RadioButtons {
        mappings: (0..15).map(note_ch0).collect(),
    };
    static ref target_select_buttons: RadioButtons = RadioButtons {
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
        )
    };
    static ref clock_select_buttons: RadioButtons = RadioButtons {
        // -1 corresponds to "internal", the rest as global clock IDs.
        mappings: (-1..8)
            .map(|clock_id| note_ch1((clock_id + CLOCK_SELECT_CONTROL_OFFSET) as u8))
            .collect(),
    };
}

pub fn map_animation_controls(device: Device, map: &mut ControlMap) {
    use AnimationTarget::*;
    use ControlMessage::*;
    use StateChange::*;
    use WaveformType::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    add(SPEED, |v| Animation(Set(Speed(bipolar_from_midi(v)))));
    add(WEIGHT, |v| Animation(Set(Weight(unipolar_from_midi(v)))));
    add(DUTY_CYCLE, |v| {
        Animation(Set(DutyCycle(unipolar_from_midi(v))))
    });
    add(SMOOTHING, |v| {
        Animation(Set(Smoothing(unipolar_from_midi(v))))
    });

    // waveform select
    add(SINE, |_| Animation(Set(Waveform(Sine))));
    add(TRIANGLE, |_| Animation(Set(Waveform(Triangle))));
    add(SQUARE, |_| Animation(Set(Waveform(Square))));
    add(SAWTOOTH, |_| Animation(Set(Waveform(Sawtooth))));

    // n periods select
    // can't do this in a loop because the callback must be fn, not a closure
    add(note_ch0(0), |_| Animation(Set(NPeriods(0))));
    add(note_ch0(1), |_| Animation(Set(NPeriods(1))));
    add(note_ch0(2), |_| Animation(Set(NPeriods(2))));
    add(note_ch0(3), |_| Animation(Set(NPeriods(3))));
    add(note_ch0(4), |_| Animation(Set(NPeriods(4))));
    add(note_ch0(5), |_| Animation(Set(NPeriods(5))));
    add(note_ch0(6), |_| Animation(Set(NPeriods(6))));
    add(note_ch0(7), |_| Animation(Set(NPeriods(7))));
    add(note_ch0(8), |_| Animation(Set(NPeriods(8))));
    add(note_ch0(9), |_| Animation(Set(NPeriods(9))));
    add(note_ch0(10), |_| Animation(Set(NPeriods(10))));
    add(note_ch0(11), |_| Animation(Set(NPeriods(11))));
    add(note_ch0(12), |_| Animation(Set(NPeriods(12))));
    add(note_ch0(13), |_| Animation(Set(NPeriods(13))));
    add(note_ch0(14), |_| Animation(Set(NPeriods(14))));
    add(note_ch0(15), |_| Animation(Set(NPeriods(15))));

    // target select
    add(ROTATION, |_| Animation(Set(Target(Rotation))));
    add(THICKNESS, |_| Animation(Set(Target(Thickness))));
    add(SIZE, |_| Animation(Set(Target(Size))));
    add(ASPECT_RATIO, |_| Animation(Set(Target(AspectRatio))));
    add(COLOR, |_| Animation(Set(Target(Color))));
    add(COLOR_SPREAD, |_| Animation(Set(Target(ColorSpread))));
    add(COLOR_PERIODICITY, |_| {
        Animation(Set(Target(ColorPeriodicity)))
    });
    add(COLOR_SATURATION, |_| {
        Animation(Set(Target(ColorSaturation)))
    });
    add(MARQUEE, |_| Animation(Set(Target(MarqueeRotation))));
    add(SEGMENTS, |_| Animation(Set(Target(Segments))));
    add(BLACKING, |_| Animation(Set(Target(Blacking))));
    add(POSITIONX, |_| Animation(Set(Target(PositionX))));
    add(POSITIONY, |_| Animation(Set(Target(PositionY))));

    // pulse/invert
    add(PULSE, |_| Animation(TogglePulse));
    add(INVERT, |_| Animation(ToggleInvert));

    // clock select
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET - 1) as u8), |_| {
        Animation(Set(ClockSource(None)))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 0) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(0)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 1) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(1)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 2) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(2)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 3) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(3)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 4) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(4)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 5) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(5)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 6) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(6)))))
    });
    add(note_ch1((CLOCK_SELECT_CONTROL_OFFSET + 7) as u8), |_| {
        Animation(Set(ClockSource(Some(ClockIdx(7)))))
    });
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
            waveform_select_buttons.select(
                match v {
                    Sine => SINE,
                    Triangle => TRIANGLE,
                    Square => SQUARE,
                    Sawtooth => SAWTOOTH,
                },
                send,
            );
        }
        NPeriods(v) => n_periods_select_buttons.select(note_ch0(v as u8), send),
        Target(v) => {
            use AnimationTarget::*;
            target_select_buttons.select(
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
            clock_select_buttons
                .select(note_ch1((index + CLOCK_SELECT_CONTROL_OFFSET) as u8), send);
        }
    }
}
