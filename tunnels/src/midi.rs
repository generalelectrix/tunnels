use std::error::Error;

use midir::{MidiInput, MidiOutput};

// Return the available ports as descriptive strings.
pub fn list_ports() -> Result<(String, String), Box<dyn Error>> {
    let input = MidiInput::new("tunnels")?;
    let inputs = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect::<Vec<String>>()
        .join("\n");
    let output = MidiOutput::new("tunnels")?;
    let outputs = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<String>>()
        .join("\n");
    Ok((inputs, outputs))
}
