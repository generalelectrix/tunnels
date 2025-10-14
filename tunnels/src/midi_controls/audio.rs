use std::time::Duration;

use tunnels_lib::number::UnipolarFloat;

use crate::{
    audio::{ControlMessage, StateChange},
    midi::{cc, event, note_on, note_on_ch1, Manager, Mapping},
    midi_controls::Device,
    show::ControlMessage::Audio,
};

use super::{unipolar_from_midi, unipolar_to_midi, ControlMap};

// Midi mappings for touch OSC.
const MONITOR: Mapping = cc(1, 0);
const MONITOR_TOGGLE: Mapping = note_on_ch1(4);
const FILTER_CUTOFF: Mapping = cc(1, 1);
const ENVELOPE_ATTACK: Mapping = cc(1, 2);
const ENVELOPE_RELEASE: Mapping = cc(1, 3);
const GAIN: Mapping = cc(1, 4);
const RESET: Mapping = note_on_ch1(5);
const IS_CLIPPING: Mapping = note_on_ch1(6);

// Midi mappings for CMD MM-1.
const CMD_MM1_VU_METER: Mapping = cc(4, 81);
const CMD_MM1_MONITOR_TOGGLE: Mapping = note_on(4, 18);

pub(crate) fn map_touch_osc_audio_controls(map: &mut ControlMap) {
    use ControlMessage::*;
    use StateChange::*;

    let mut add = |mapping, creator| map.add(Device::TouchOsc, mapping, creator);

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

pub(crate) fn map_cmd_mm1_audio_controls(map: &mut ControlMap) {
    use ControlMessage::*;

    let mut add = |mapping, creator| map.add(Device::BehringerCmdMM1, mapping, creator);

    add(CMD_MM1_MONITOR_TOGGLE, Box::new(|_| Audio(ToggleMonitor)));
}

/// Emit midi messages to update UIs given the provided state change.
pub(crate) fn update_audio_control(sc: StateChange, manager: &mut Manager<Device>) {
    use StateChange::*;

    let mut send = |event| {
        manager.send(&Device::TouchOsc, event);
    };

    match sc {
        EnvelopeValue(v) => {
            send(event(MONITOR, unipolar_to_midi(v)));
            manager.send(
                &Device::BehringerCmdMM1,
                event(CMD_MM1_VU_METER, 48 + (v.val() * 15.) as u8),
            );
        }
        Monitor(v) => {
            send(event(MONITOR_TOGGLE, v as u8));
            manager.send(
                &Device::BehringerCmdMM1,
                event(CMD_MM1_MONITOR_TOGGLE, v as u8),
            );
        }
        FilterCutoff(v) => send(event(FILTER_CUTOFF, filter_to_midi(v))),
        EnvelopeAttack(v) => send(event(ENVELOPE_ATTACK, envelope_edge_to_midi(v))),
        EnvelopeRelease(v) => send(event(ENVELOPE_RELEASE, envelope_edge_to_midi(v))),
        Gain(v) => send(event(GAIN, gain_to_midi(v))),
        IsClipping(v) => send(event(IS_CLIPPING, v as u8)),
    }
}

/// Get midi value plus 1, in milliseconds.
pub fn envelope_edge_from_midi(v: u8) -> Duration {
    Duration::from_millis(v as u64 + 1)
}

/// Clamp duration in integer milliseconds to midi range.
pub fn envelope_edge_to_midi(d: Duration) -> u8 {
    let clamped = d.as_millis().clamp(1, 128);
    (clamped - 1) as u8
}

// Crude filter control - linear, roughly 1kHz range, "0" is 40 Hz.
// FIXME: make this logarithmic

const FILTER_LOWER_BOUND: f64 = 40.;
const FILTER_SCALE: f64 = 1000.;

pub fn filter_from_midi(v: u8) -> f32 {
    (unipolar_from_midi(v).val() * FILTER_SCALE + FILTER_LOWER_BOUND) as f32
}

pub fn filter_to_midi(f: f32) -> u8 {
    unipolar_to_midi(UnipolarFloat::new(
        ((f as f64) - FILTER_LOWER_BOUND) / FILTER_SCALE,
    ))
}

// Set gain as a unipolar knob, scaled by 20, interpreted as dB.

pub fn gain_from_midi(v: u8) -> f64 {
    let gain_db = 20. * unipolar_from_midi(v).val();
    (10_f64).powf(gain_db / 20.)
}

pub fn gain_to_midi(g: f64) -> u8 {
    let gain_db = 20. * g.log10();
    unipolar_to_midi(UnipolarFloat::new(gain_db / 20.))
}
