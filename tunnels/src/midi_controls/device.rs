use crate::midi::{Event, EventType, MidiOutput, Mapping};
use crate::show::ControlMessage;
use anyhow::Result;
use log::debug;
use midi_harness::{InitMidiDevice, Output};

/// The input MIDI device types that tunnels can work with.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Device {
    AkaiApc40,
    AkaiApc20,
    TouchOsc,
    BehringerCmdMM1,
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.device_name())
    }
}

impl Device {
    pub fn all() -> Vec<Self> {
        vec![
            Self::AkaiApc40,
            Self::TouchOsc,
            Self::BehringerCmdMM1,
            // TODO: update support for Apc20.
        ]
    }
}

pub trait MidiDevice: PartialEq + Sized + Send + Clone + InitMidiDevice + 'static {
    /// Return the name of the midi device we should look for.
    fn device_name(&self) -> &str;
}

impl InitMidiDevice for Device {
    /// Perform device-specific midi initialization.
    fn init_midi(&self, out: &mut dyn Output) -> Result<()> {
        match *self {
            Self::AkaiApc40 => init_apc_40(out),
            Self::AkaiApc20 => init_apc_20(out),
            Self::TouchOsc => Ok(()),
            Self::BehringerCmdMM1 => Ok(()),
        }
    }
}

impl MidiDevice for Device {
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

fn init_apc_40(out: &mut dyn Output) -> Result<()> {
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

pub trait MidiHandler {
    fn interpret(&self, event: &Event) -> Option<ControlMessage>;
    fn emit_tunnel(&self, _sc: &crate::tunnel::StateChange, _output: &mut impl MidiOutput) {}
    fn emit_animation(
        &self,
        _sc: &crate::animation::StateChange,
        _output: &mut impl MidiOutput,
    ) {
    }
    fn emit_animation_target(
        &self,
        _sc: &crate::animation_target::AnimationTarget,
        _output: &mut impl MidiOutput,
    ) {
    }
    fn emit_mixer(&self, _sc: &crate::mixer::StateChange, _output: &mut impl MidiOutput) {}
    fn emit_clock(
        &self,
        _sc: &crate::clock_bank::StateChange,
        _output: &mut impl MidiOutput,
    ) {
    }
    fn emit_master_ui(
        &self,
        _sc: &crate::master_ui::StateChange,
        _output: &mut impl MidiOutput,
    ) {
    }
    fn emit_audio(&self, _sc: &crate::audio::StateChange, _output: &mut impl MidiOutput) {}
}

impl MidiHandler for Device {
    fn interpret(&self, event: &Event) -> Option<ControlMessage> {
        match self {
            Device::AkaiApc40 => None
                .or_else(|| super::tunnel::interpret(event))
                .or_else(|| super::animation::interpret(event))
                .or_else(|| super::mixer::interpret(event, 0))
                .or_else(|| super::master_ui::interpret(event, 0)),
            Device::AkaiApc20 => None
                .or_else(|| super::mixer::interpret(event, 1))
                .or_else(|| super::master_ui::interpret(event, 1)),
            Device::TouchOsc => None
                .or_else(|| super::tunnel::interpret(event))
                .or_else(|| super::animation::interpret(event))
                .or_else(|| super::animation_target::interpret(event))
                .or_else(|| super::mixer::interpret(event, 0))
                .or_else(|| super::master_ui::interpret(event, 0))
                .or_else(|| super::clock::interpret_touchosc(event))
                .or_else(|| super::audio::interpret_touchosc(event)),
            Device::BehringerCmdMM1 => None
                .or_else(|| super::clock::interpret_cmdmm1(event))
                .or_else(|| super::audio::interpret_cmdmm1(event)),
        }
    }
}

pub fn init_apc_20(out: &mut dyn Output) -> Result<()> {
    // put into ableton (full control) mode
    debug!("Sending APC20 sysex mode command.");
    out.send_raw(&[
        0xF0, 0x47, 0x7F, 0x7B, 0x60, 0x00, 0x04, 0x42, 0x08, 0x02, 0x01, 0xF7,
    ])?;
    Ok(())
}
