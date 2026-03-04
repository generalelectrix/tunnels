//! Helpers for selecting ports from a CLI.
use anyhow::Result;
use midir::{MidiInput, MidiOutput};

use crate::DeviceId;

/// Return the available ports by name.
pub fn list_ports() -> Result<(Vec<MidiPortSpec>, Vec<MidiPortSpec>)> {
    let input = MidiInput::new("list_ports")?;
    let inputs = input
        .ports()
        .iter()
        .filter_map(|p| {
            input.port_name(p).ok().map(|name| MidiPortSpec {
                name,
                id: DeviceId(p.id()),
            })
        })
        .collect();
    let output = MidiOutput::new("list_ports")?;
    let outputs = output
        .ports()
        .iter()
        .filter_map(|p| {
            output.port_name(p).ok().map(|name| MidiPortSpec {
                name,
                id: DeviceId(p.id()),
            })
        })
        .collect();
    Ok((inputs, outputs))
}

/// A specified MIDI port, including both the unique ID and name.
#[derive(Clone)]
pub struct MidiPortSpec {
    pub id: DeviceId,
    pub name: String,
}
