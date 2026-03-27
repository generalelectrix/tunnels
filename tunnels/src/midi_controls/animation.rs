use crate::{
    animation::{ControlMessage, StateChange, Waveform as WaveformType},
    clock_bank::{ClockIdxExt, N_CLOCKS},
    midi::{cc_ch0, event, note_on_ch0, note_on_ch1, Event, EventType, MidiOutput, Mapping},
    midi_controls::Device,
    show::ControlMessage::Animation,
};
use lazy_static::lazy_static;

use super::{
    bipolar_from_midi, bipolar_to_midi, unipolar_from_midi, unipolar_to_midi, RadioButtons,
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
const NOISE: Mapping = note_on_ch0(28);
const CONSTANT: Mapping = note_on_ch0(29);

// These buttons are on channel 1 instead of 0 as we ran out of space on channel 1.
const PULSE: Mapping = note_on_ch1(0);
const INVERT: Mapping = note_on_ch1(1);
const USE_AUDIO_SIZE: Mapping = note_on_ch1(2);
const USE_AUDIO_SPEED: Mapping = note_on_ch1(3);
const STANDING: Mapping = note_on_ch1(7);

const CLOCK_SELECT_CONTROL_OFFSET: i32 = 112;

lazy_static! {
    static ref WAVEFORM_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: vec!(SINE, TRIANGLE, SQUARE, SAWTOOTH, NOISE, CONSTANT), off: 0, on: 1,
    };
    static ref N_PERIODS_SELECT_BUTTONS: RadioButtons = RadioButtons {
        mappings: (0..15).map(note_on_ch0).collect(), off: 0, on: 1,
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

pub fn interpret(event: &Event) -> Option<crate::show::ControlMessage> {
    use ControlMessage::*;
    use StateChange::*;
    use WaveformType::*;
    let v = event.value;
    Some(match event.mapping {
        SPEED => Animation(Set(Speed(bipolar_from_midi(v)))),
        SIZE => Animation(Set(Size(unipolar_from_midi(v)))),
        DUTY_CYCLE => Animation(Set(DutyCycle(unipolar_from_midi(v)))),
        SMOOTHING => Animation(Set(Smoothing(unipolar_from_midi(v)))),
        SINE => Animation(Set(Waveform(Sine))),
        TRIANGLE => Animation(Set(Waveform(Triangle))),
        SQUARE => Animation(Set(Waveform(Square))),
        SAWTOOTH => Animation(Set(Waveform(Sawtooth))),
        NOISE => Animation(Set(Waveform(Noise))),
        CONSTANT => Animation(Set(Waveform(Constant))),
        PULSE => Animation(TogglePulse),
        INVERT => Animation(ToggleInvert),
        STANDING => Animation(ToggleStanding),
        USE_AUDIO_SIZE => Animation(ToggleUseAudioSize),
        USE_AUDIO_SPEED => Animation(ToggleUseAudioSpeed),
        m if m.event_type == EventType::NoteOn
            && m.channel == 0
            && m.control < 16 =>
        {
            Animation(Set(NPeriods(m.control as u16)))
        }
        m if m.event_type == EventType::NoteOn
            && m.channel == 0
            && m.control >= (CLOCK_SELECT_CONTROL_OFFSET - 1) as u8
            && m.control < (CLOCK_SELECT_CONTROL_OFFSET + N_CLOCKS as i32) as u8 =>
        {
            let clock_id = m.control as i32 - CLOCK_SELECT_CONTROL_OFFSET;
            if clock_id < 0 {
                Animation(Set(ClockSource(None)))
            } else {
                Animation(SetClockSource(Some(ClockIdxExt(clock_id as usize))))
            }
        }
        _ => return None,
    })
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_animation_control(sc: StateChange, manager: &mut impl MidiOutput) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(&Device::AkaiApc40, event);
        manager.send(&Device::TouchOsc, event);
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
                    Noise => NOISE,
                    Constant => CONSTANT,
                },
                send,
            );
        }
        NPeriods(v) => N_PERIODS_SELECT_BUTTONS.select(note_on_ch0(v as u8), send),
        Invert(v) => send(event(INVERT, v as u8)),
        Standing(v) => send(event(STANDING, v as u8)),
        Pulse(v) => send(event(PULSE, v as u8)),
        ClockSource(v) => {
            let index = match v {
                Some(source) => usize::from(source) as i32,
                None => -1,
            };
            CLOCK_SELECT_BUTTONS.select(
                note_on_ch0((index + CLOCK_SELECT_CONTROL_OFFSET) as u8),
                send,
            );
        }
        UseAudioSize(v) => send(event(USE_AUDIO_SIZE, v as u8)),
        UseAudioSpeed(v) => send(event(USE_AUDIO_SPEED, v as u8)),
    }
}
