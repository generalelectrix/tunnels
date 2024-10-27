use std::fmt;

use crate::midi::{Event, EventType, Mapping, Output};
use log::debug;
use midir::SendError;

/// The input MIDI device types that tunnels can work with.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Device {
    AkaiApc40,
    AkaiApc20,
    TouchOsc,
    BehringerCmdMM1,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::AkaiApc40 => "Akai APC40",
                Self::AkaiApc20 => "Akai APC20",
                Self::TouchOsc => "Touch OSC",
                Self::BehringerCmdMM1 => "Behringer CMD MM-1",
            }
        )
    }
}

pub trait MidiDevice: PartialEq + Sized + Send + Clone {
    /// Return the name of the midi device we should look for.
    fn device_name(&self) -> &str;

    /// Perform device-specific midi initialization.
    #[allow(unused)]
    fn init_midi(&self, out: &mut Output<Self>) -> Result<(), SendError> {
        Ok(())
    }
}

impl MidiDevice for Device {
    /// Perform device-specific midi initialization.
    fn init_midi(&self, out: &mut Output<Device>) -> Result<(), SendError> {
        match *self {
            Self::AkaiApc40 => init_apc_40(out),
            Self::AkaiApc20 => init_apc_20(out),
            Self::TouchOsc => Ok(()),
            Self::BehringerCmdMM1 => Ok(()),
        }
    }

    /// Return the name of the midi device we should look for.
    fn device_name(&self) -> &str {
        match *self {
            Self::AkaiApc40 => "Akai APC40",
            Self::AkaiApc20 => "Akai APC20",
            Self::TouchOsc => "TouchOSC Bridge",
            Self::BehringerCmdMM1 => "CMD MM-1",
        }
    }
}

fn init_apc_40(out: &mut Output<impl MidiDevice>) -> Result<(), SendError> {
    // put into ableton (full control) mode
    debug!("Sending APC40 sysex mode command.");
    out.send_raw(&[
        0xF0, 0x47, 0x00, 0x73, 0x60, 0x00, 0x04, 0x42, 0x08, 0x04, 0x01, 0xF7,
    ])?;

    let knob_off = 0;
    let knob_single = 1;
    let knob_volume = 2;
    let knob_pan = 3;

    let mut ring_settings = Vec::new();
    let mut add_ring_setting = |control: u8, value: u8| {
        ring_settings.push(Event {
            mapping: Mapping {
                event_type: EventType::ControlChange,
                channel: 0,
                control,
            },
            value,
        });
    };

    // top knobs
    add_ring_setting(0x38, knob_pan);
    add_ring_setting(0x39, knob_volume);
    add_ring_setting(0x3A, knob_single);
    add_ring_setting(0x3B, knob_single);

    add_ring_setting(0x3C, knob_pan);
    add_ring_setting(0x3D, knob_volume);
    add_ring_setting(0x3E, knob_single);
    add_ring_setting(0x3F, knob_off);

    // bottom knobs
    add_ring_setting(0x18, knob_single);
    add_ring_setting(0x19, knob_single);
    add_ring_setting(0x1A, knob_volume);
    add_ring_setting(0x1B, knob_volume);
    add_ring_setting(0x1C, knob_pan);
    add_ring_setting(0x1D, knob_volume);
    add_ring_setting(0x1E, knob_volume);
    add_ring_setting(0x1F, knob_single);

    debug!("Setting LED behaviors.");
    for setting in ring_settings {
        out.send(setting)?;
    }
    Ok(())
}

fn init_apc_20(out: &mut Output<impl MidiDevice>) -> Result<(), SendError> {
    // put into ableton (full control) mode
    debug!("Sending APC20 sysex mode command.");
    out.send_raw(&[
        0xF0, 0x47, 0x7F, 0x7B, 0x60, 0x00, 0x04, 0x42, 0x08, 0x02, 0x01, 0xF7,
    ])
}
