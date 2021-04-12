use midir::{
    MidiIO, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort, SendError,
};
use simple_error::bail;
use std::error::Error;

pub enum EventType {
    NoteOn,
    NoteOff,
    ControlChange,
}

pub struct Mapping {
    pub event_type: EventType,
    pub channel: u8,
    pub control: u8,
}

pub struct Event {
    pub mapping: Mapping,
    pub value: u8,
}

impl Mapping {
    pub fn event_type(&self) -> u8 {
        match *self {
            Mapping::ControlChange {
                chan: _,
                control: _,
            } => 11 << 4,
            Mapping::NoteOn {
                chan: _,
                control: _,
            } => 9 << 4,
            Mapping::NoteOff {
                chan: _,
                control: _,
            } => 8 << 4,
        }
    }
}

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

fn get_named_port<T: MidiIO>(source: &T, name: &str) -> Result<T::Port, Box<dyn Error>> {
    for port in source.ports() {
        if let Ok(this_name) = source.port_name(&port) {
            if this_name == name {
                return Ok(port);
            }
        }
    }
    bail!("no port found with name {}", name);
}

pub struct Output {
    name: String,
    conn: MidiOutputConnection,
}

impl Output {
    pub fn new(name: String) -> Result<Self, Box<dyn Error>> {
        let output = MidiOutput::new("tunnels")?;
        let port = get_named_port(&output, &name)?;
        let conn = output.connect(&port, &name)?;
        Ok(Self { name, conn })
    }

    pub fn send(&self, event: Event) -> Result<(), SendError> {
        let msg: [u8; 3] = [0; 3];
        msg[0] = match event.mapping.event_type {
            EventType::ControlChange => 11 << 4,
            EventType::NoteOn => 9 << 4,
            EventType::NoteOff => 8 << 4,
        } + event.mapping.channel;
        msg[1] = event.mapping.control;
        msg[2] = event.value;
        self.conn.send(&msg)
    }
}
