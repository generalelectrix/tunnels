use anyhow::{anyhow, bail, Result};
use log::{debug, error};
use midi_harness::{DeviceId, DeviceManager, MidiHandler};
use midir::{MidiIO, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, SendError};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::mpsc::Sender};
use tunnels_lib::prompt::{prompt_bool, prompt_indexed_value};

use crate::{
    control::ControlEvent,
    midi_controls::{Device, MidiDevice},
};

pub use midi_harness::event::*;

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

/// Handle MIDI events by forwarding to a channel.
#[derive(Clone)]
struct ControlEventHandler(Sender<ControlEvent>);

impl MidiHandler<Device> for ControlEventHandler {
    fn handle(&self, event: Event, device: &Device) {
        let _ = self.0.send(ControlEvent::Midi((*device, event)));
    }
}

/// Maintain midi inputs and outputs.
/// Provide synchronous dispatch for outgoing messages based on device type.
pub struct Manager {
    manager: DeviceManager<Device, ControlEventHandler>,
}

impl Manager {
    /// Initialize the manager.
    pub fn new(send: Sender<ControlEvent>) -> Self {
        Self {
            manager: DeviceManager::new(ControlEventHandler(send)),
        }
    }

    /// Add a device to the manager given input and output port names.
    pub fn add_device(&mut self, slot_name: String, spec: DeviceSpec<Device>) -> Result<()> {
        self.manager.add_slot(slot_name.clone(), spec.device)?;
        self.manager.connect_input(&slot_name, spec.input_id)?;
        self.manager.connect_output(&slot_name, spec.output_id)?;
        Ok(())
    }

    /// Send a message to the specified device type.
    /// Error conditions are logged rather than returned.
    pub fn send(&mut self, device: &Device, event: Event) {
        self.manager.visit_outputs(|d, mut output| {
            if d == device {
                if let Err(e) = output.send(event) {
                    error!("Failed to send midi event to {}: {}.", output.name(), e);
                }
            }
        });
    }
}

/// Wrapper struct for the data needed to describe a device to connect to.
#[derive(Clone, Debug)]
pub struct DeviceSpec<D> {
    pub device: D,
    pub input_id: DeviceId,
    pub output_id: DeviceId,
}

/// Prompt the user to configure midi devices.
pub fn prompt_midi<D: MidiDevice>(
    input_ports: &[MidiPortSpec],
    output_ports: &[MidiPortSpec],
    known_device_types: Vec<D>,
) -> Result<Vec<DeviceSpec<D>>> {
    let mut devices = Vec::new();
    println!("Available devices:");
    for (i, port) in input_ports.iter().enumerate() {
        println!("{i}: {}", port.name);
    }
    for (i, port) in output_ports.iter().enumerate() {
        println!("{i}: {}", port.name);
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

#[derive(Clone)]
pub struct MidiPortSpec {
    pub id: DeviceId,
    pub name: String,
}

/// Prompt the user to select input and output ports for a device.
fn prompt_input_output<D: MidiDevice>(
    device: D,
    input_ports: &[MidiPortSpec],
    output_ports: &[MidiPortSpec],
) -> Result<DeviceSpec<D>> {
    let name = device.device_name().to_string();
    if let Some(input) = input_ports.iter().find(|p| p.name == name) {
        if let Some(output) = output_ports.iter().find(|p| p.name == name) {
            return Ok(DeviceSpec {
                device,
                input_id: input.id.clone(),
                output_id: output.id.clone(),
            });
        }
    }
    println!("Didn't find a device of the expected name. Please manually select input and output.");
    let input_id = prompt_indexed_value("Input port:", input_ports)?.id;
    let output_id = prompt_indexed_value("Output port:", output_ports)?.id;
    Ok(DeviceSpec {
        device,
        input_id,
        output_id,
    })
}
