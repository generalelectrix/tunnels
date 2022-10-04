use std::{
    cmp::{max, min},
    time::Duration,
};

use tunnels_lib::number::UnipolarFloat;

use crate::{
    audio::ControlMessage,
    audio::StateChange,
    midi::{cc, event, note_on_ch1, Manager, Mapping},
    midi_controls::Device,
    show::ControlMessage::Audio,
};

use super::{unipolar_from_midi, unipolar_to_midi, ControlMap};

const MONITOR: Mapping = cc(1, 0);
const MONITOR_TOGGLE: Mapping = note_on_ch1(4);
const FILTER_CUTOFF: Mapping = cc(1, 1);
const ENVELOPE_ATTACK: Mapping = cc(1, 2);
const ENVELOPE_RELEASE: Mapping = cc(1, 3);
const GAIN: Mapping = cc(1, 4);
const RESET: Mapping = note_on_ch1(5);
const IS_CLIPPING: Mapping = note_on_ch1(6);

pub fn map_audio_controls(device: Device, map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;

    let mut add = |mapping, creator| map.add(device, mapping, creator);

    add(MONITOR_TOGGLE, Box::new(|_| Audio(ToggleMonitor)));
    add(
        FILTER_CUTOFF,
        Box::new(|v| Audio(Set(FilterCutoff(filter_from_midi(v))))),
    );
    add(
        ENVELOPE_ATTACK,
        Box::new(|v| Audio(Set(EnvelopeAttack(envelope_edge_from_midi(v))))),
    );
    add(
        ENVELOPE_RELEASE,
        Box::new(|v| Audio(Set(EnvelopeRelease(envelope_edge_from_midi(v))))),
    );
    add(RESET, Box::new(|_| Audio(ResetParameters)));
    add(GAIN, Box::new(|v| Audio(Set(Gain(gain_from_midi(v))))));
}

/// Emit midi messages to update UIs given the provided state change.
pub fn update_audio_control(sc: StateChange, manager: &mut Manager) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(Device::TouchOsc, event);
    };

    match sc {
        EnvelopeValue(v) => send(event(MONITOR, unipolar_to_midi(v))),
        Monitor(v) => send(event(MONITOR_TOGGLE, v as u8)),
        FilterCutoff(v) => send(event(FILTER_CUTOFF, filter_to_midi(v))),
        EnvelopeAttack(v) => send(event(ENVELOPE_ATTACK, envelope_edge_to_midi(v))),
        EnvelopeRelease(v) => send(event(ENVELOPE_RELEASE, envelope_edge_to_midi(v))),
        Gain(v) => send(event(GAIN, gain_to_midi(v))),
        IsClipping(v) => send(event(IS_CLIPPING, v as u8)),
    }
}

/// Get midi value plus 1, in milliseconds.
fn envelope_edge_from_midi(v: u8) -> Duration {
    Duration::from_millis(v as u64 + 1)
}

/// Clamp duration in integer milliseconds to midi range.
fn envelope_edge_to_midi(d: Duration) -> u8 {
    let clamped = max(min(d.as_millis(), 128), 1);
    (clamped - 1) as u8
}

// Crude filter control - linear, roughly 1kHz range, "0" is 40 Hz.
// FIXME: make this logarithmic

const FILTER_LOWER_BOUND: f64 = 40.;
const FILTER_SCALE: f64 = 1000.;

fn filter_from_midi(v: u8) -> f32 {
    (unipolar_from_midi(v).val() * FILTER_SCALE + FILTER_LOWER_BOUND) as f32
}

fn filter_to_midi(f: f32) -> u8 {
    unipolar_to_midi(UnipolarFloat::new(
        ((f as f64) - FILTER_LOWER_BOUND) / FILTER_SCALE,
    ))
}

// Set gain as a unipolar knob, scaled by 10, interpreted as dB.

fn gain_from_midi(v: u8) -> f64 {
    let gain_db = 10. * unipolar_from_midi(v).val();
    (10_f64).powf(gain_db / 20.)
}

fn gain_to_midi(g: f64) -> u8 {
    let gain_db = 20. * g.log10();
    unipolar_to_midi(UnipolarFloat::new(gain_db / 10.))
}
