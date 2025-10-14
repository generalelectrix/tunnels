use anyhow::{anyhow, bail, Result};
use log::{debug, error};
use midir::{MidiIO, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, SendError};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::mpsc::Sender};
use tunnels_lib::prompt::{prompt_bool, prompt_indexed_value};

use crate::{
    control::ControlEvent,
    midi_controls::{Device, MidiDevice},
};

/// Specification for what type of midi event.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventType {
    NoteOn,
    NoteOff,
    ControlChange,
}

/// A specification of a midi mapping.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Mapping {
    pub event_type: EventType,
    pub channel: u8,
    pub control: u8,
}

impl fmt::Display for Mapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}:{}",
            match self.event_type {
                EventType::NoteOn => "NoteOn ",
                EventType::NoteOff => "NoteOff",
                EventType::ControlChange => "CntChng",
            },
            self.channel,
            self.control
        )
    }
}

/// Helper constructor for a note on mapping.
pub const fn note_on(channel: u8, control: u8) -> Mapping {
    Mapping {
        event_type: EventType::NoteOn,
        channel,
        control,
    }
}

/// Helper constructor for a note off mapping.
pub const fn note_off(channel: u8, control: u8) -> Mapping {
    Mapping {
        event_type: EventType::NoteOff,
        channel,
        control,
    }
}

/// Helper constructor - most controls are on channel 0.
pub const fn note_on_ch0(control: u8) -> Mapping {
    note_on(0, control)
}

/// Helper constructor - other relevant special case is channel 1.
pub const fn note_on_ch1(control: u8) -> Mapping {
    note_on(1, control)
}

/// Helper constructor for a control change mapping.
pub const fn cc(channel: u8, control: u8) -> Mapping {
    Mapping {
        event_type: EventType::ControlChange,
        channel,
        control,
    }
}

/// Helper constructor - most controls are on channel 0.
pub const fn cc_ch0(control: u8) -> Mapping {
    cc(0, control)
}

/// A fully-specified midi event.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct Event {
    pub mapping: Mapping,
    pub value: u8,
}

/// Helper constructor for a midi event.
pub const fn event(mapping: Mapping, value: u8) -> Event {
    Event { mapping, value }
}

// Return the available ports by name,
pub fn list_ports() -> Result<(Vec<String>, Vec<String>)> {
    let input = MidiInput::new("tunnels")?;
    let inputs = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect::<Vec<String>>();
    let output = MidiOutput::new("tunnels")?;
    let outputs = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<String>>();
    Ok((inputs, outputs))
}

fn get_named_port<T: MidiIO>(source: &T, name: &str) -> Result<T::Port> {
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
    pub fn new(name: String) -> Result<Self> {
        let output = MidiOutput::new(&name)?;
        let port = get_named_port(&output, &name)?;
        let conn = output
            .connect(&port, &name)
            .map_err(|err| anyhow!("failed to connect to midi output: {err}"))?;
        Ok(Self { conn, name })
    }

    pub fn send(&mut self, event: Event) -> Result<(), SendError> {
        let mut msg: [u8; 3] = [0; 3];
        msg[0] = match event.mapping.event_type {
            EventType::ControlChange => 11 << 4,
            EventType::NoteOn => 9 << 4,
            EventType::NoteOff => 8 << 4,
        } + event.mapping.channel;
        msg[1] = event.mapping.control;
        msg[2] = event.value;
        self.conn.send(&msg)
    }

    pub fn send_raw(&mut self, msg: &[u8]) -> Result<(), SendError> {
        self.conn.send(msg)
    }
}

pub struct Input {
    _conn: MidiInputConnection<()>,
}

pub trait CreateControlEvent<D> {
    fn from_event(event: Event, device: D) -> Self;
}

impl CreateControlEvent<Device> for ControlEvent {
    fn from_event(event: Event, device: Device) -> Self {
        ControlEvent::Midi((device, event))
    }
}

impl Input {
    pub fn new<D, E>(name: String, device: D, sender: Sender<E>) -> Result<Self>
    where
        D: Send + 'static + Clone,
        E: CreateControlEvent<D> + Send + 'static,
    {
        let input = MidiInput::new(&name)?;
        let port = get_named_port(&input, &name)?;
        let handler_name = name.clone();

        let conn = input
            .connect(
                &port,
                &name,
                move |_, msg: &[u8], _| {
                    let control = msg[1];
                    let value = msg[2];
                    let event_type = match msg[0] >> 4 {
                        // Most midi devices just send NoteOn with a velocity of 0 for NoteOff.
                        8 | 9 if value == 0 => EventType::NoteOff,
                        9 => EventType::NoteOn,
                        11 => EventType::ControlChange,
                        other => {
                            debug!(
                                "Ignoring midi input event on {handler_name} of unimplemented type {other}."
                            );
                            return;
                        }
                    };
                    let channel = msg[0] & 15;
                    sender
                        .send(E::from_event(
                            Event {
                                mapping: Mapping {
                                    event_type,
                                    channel,
                                    control,
                                },
                                value,
                            },
                            device.clone(),
                        ))
                        .unwrap();
                },
                (),
            )
            .map_err(|err| anyhow!("failed to connect to midi input: {err}"))?;
        Ok(Input { _conn: conn })
    }
}

/// Maintain midi inputs and outputs.
/// Provide synchronous dispatch for outgoing messages based on device type.
pub struct Manager<D: MidiDevice> {
    inputs: Vec<Input>,
    outputs: Vec<(D, Output)>,
}

impl<D: MidiDevice> Default for Manager<D> {
    fn default() -> Self {
        Self {
            inputs: Default::default(),
            outputs: Default::default(),
        }
    }
}

impl<D: MidiDevice + 'static> Manager<D> {
    /// Add a device to the manager given input and output port names.
    pub fn add_device(
        &mut self,
        spec: DeviceSpec<D>,
        send: Sender<impl CreateControlEvent<D> + Send + 'static>,
    ) -> Result<()> {
        let input = Input::new(spec.input_port_name, spec.device.clone(), send)?;
        let mut output = Output::new(spec.output_port_name)?;

        // Send initialization commands to the device.
        spec.device.init_midi(&mut output)?;

        self.inputs.push(input);
        self.outputs.push((spec.device, output));
        Ok(())
    }

    /// Send a message to the specified device type.
    /// Error conditions are logged rather than returned.
    pub fn send(&mut self, device: &D, event: Event) {
        for (d, output) in &mut self.outputs {
            if d == device {
                if let Err(e) = output.send(event) {
                    error!("Failed to send midi event to {}: {}.", output.name, e);
                }
            }
        }
    }

    /// Return an iterator over all outputs.
    pub fn outputs(&mut self) -> impl Iterator<Item = &mut (D, Output)> {
        self.outputs.iter_mut()
    }
}

/// Wrapper struct for the data needed to describe a device to connect to.
#[derive(Clone, Debug)]
pub struct DeviceSpec<D> {
    pub device: D,
    pub input_port_name: String,
    pub output_port_name: String,
}

/// Prompt the user to configure midi devices.
pub fn prompt_midi<D: MidiDevice>(
    input_ports: &[String],
    output_ports: &[String],
    known_device_types: Vec<D>,
) -> Result<Vec<DeviceSpec<D>>> {
    let mut devices = Vec::new();
    println!("Available devices:");
    for (i, port) in input_ports.iter().enumerate() {
        println!("{i}: {port}");
    }
    for (i, port) in output_ports.iter().enumerate() {
        println!("{i}: {port}");
    }
    println!();

    let mut add_device = |device: D| -> Result<()> {
        if prompt_bool(&format!("Use {}?", device.device_name()))? {
            devices.push(prompt_input_output(device, input_ports, output_ports)?);
        }
        Ok(())
    };

    for d in known_device_types {
        add_device(d)?;
    }

    Ok(devices)
}

/// Prompt the user to select input and output ports for a device.
fn prompt_input_output<D: MidiDevice>(
    device: D,
    input_ports: &[String],
    output_ports: &[String],
) -> Result<DeviceSpec<D>> {
    let name = device.device_name().to_string();
    if input_ports.contains(&name) && output_ports.contains(&name) {
        return Ok(DeviceSpec {
            device,
            input_port_name: name.to_string(),
            output_port_name: name.to_string(),
        });
    }
    println!("Didn't find a device of the expected name. Please manually select input and output.");
    let input_port_name = prompt_indexed_value("Input port:", input_ports)?;
    let output_port_name = prompt_indexed_value("Output port:", output_ports)?;
    Ok(DeviceSpec {
        device,
        input_port_name,
        output_port_name,
    })
}
