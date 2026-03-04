use anyhow::Result;
use log::{error, info};
use midi_harness::{
    DeviceChange, DeviceId, DeviceManager, HandleDeviceChange, MidiHandler, MidiPortSpec,
};
use std::sync::mpsc::Sender;
use tunnels_lib::prompt::{prompt_bool, prompt_indexed_value};

use crate::{
    control::ControlEvent,
    midi_controls::{Device, MidiDevice},
};

pub use midi_harness::event::*;
pub use midi_harness::list_ports;

/// Handle MIDI events by forwarding to a channel.
#[derive(Clone)]
struct ControlEventHandler(Sender<ControlEvent>);

impl MidiHandler<Device> for ControlEventHandler {
    fn handle(&self, event: Event, device: &Device) {
        self.0.send(ControlEvent::Midi((*device, event))).unwrap();
    }
}

impl HandleDeviceChange for ControlEventHandler {
    fn on_device_change(&self, change: Result<DeviceChange>) {
        self.0.send(ControlEvent::MidiDevice(change)).unwrap();
    }
}
/// Maintain midi inputs and outputs.
/// Provide synchronous dispatch for outgoing messages based on device type.
pub struct Manager {
    manager: DeviceManager<Device, ControlEventHandler>,
}

impl Manager {
    /// Initialize the manager.
    pub fn new(send: Sender<ControlEvent>) -> Result<Self> {
        Ok(Self {
            manager: DeviceManager::new(ControlEventHandler(send))?,
        })
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

    /// Handle a device appearing or disappearing.
    pub fn handle_device_change(&mut self, change: DeviceChange) -> Result<()> {
        self.manager.handle_device_change(change)
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
