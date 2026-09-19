use crate::{
    audio::{ControlMessage, StateChange},
    midi::{Mapping, MidiOutput, cc, event, note_on},
    midi_controls::Device,
    show::ControlMessage::Audio,
};

use crate::midi::Event as MidiEvent;

// Midi mappings for CMD MM-1.
const CMD_MM1_VU_METER: Mapping = cc(4, 81);
const CMD_MM1_MONITOR_TOGGLE: Mapping = note_on(4, 18);

pub fn interpret_cmdmm1(event: &MidiEvent) -> Option<crate::show::ControlMessage> {
    use ControlMessage::*;
    Some(match event.mapping {
        CMD_MM1_MONITOR_TOGGLE => Audio(ToggleMonitor),
        _ => return None,
    })
}

/// Emit midi messages to update UIs given the provided state change.
pub(crate) fn update_audio_control(sc: StateChange, manager: &mut impl MidiOutput) {
    use StateChange::*;

    // Audio metering is global; address the first (canonical) clock wing.
    let mut send = |event| {
        manager.send(&Device::BehringerCmdMM1 { channel_offset: 0 }, event);
    };

    match sc {
        EnvelopeValue(v) => send(event(CMD_MM1_VU_METER, 48 + (v.val() * 15.) as u8)),
        Monitor(v) => send(event(CMD_MM1_MONITOR_TOGGLE, v as u8)),
        EnvelopeAttack(_)
        | EnvelopeRelease(_)
        | OutputSmoothing(_)
        | InputGain(_)
        | ActiveBand(_)
        | NormFloorHalflife(_)
        | NormCeilingHalflife(_)
        | NormFloorMode(_) => {}
    }
}
