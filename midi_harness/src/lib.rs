//! MIDI device management to plug into an event-driven environment.
mod device_change;
pub mod event;

use anyhow::{Context, Result, bail};
pub use device_change::initialize;
use log::debug;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, SendError};

use crate::{
    device_change::DeviceId,
    event::{Event, EventType, Mapping, ParseError},
};

pub struct DeviceManager<D, F>
where
    D: 'static + Clone,
    F: Fn(Event, &D) + Clone,
{
    slots: Vec<DeviceSlot<D>>,
    /// Callback that will be called by a MIDI input on message arrival.
    ///
    /// Accepts the MIDI Event, and will also be handed your device model to be
    /// used to interpret the message. This callback should do something with
    /// the result, such as push it onto a channel.
    proc_input: F,
}

impl<D, F> DeviceManager<D, F>
where
    D: 'static + Clone,
    F: Fn(Event, &D) + Clone,
{
    /// Call the provided closure on each connected output.
    /// The attached model and the MIDI output are provided.
    pub fn visit_outputs(&mut self, visitor: impl Fn(&D, OutputPort)) {
        for slot in &mut self.slots {
            let Some(device) = &mut slot.device else {
                continue;
            };
            let Some(output) = &mut device.output else {
                continue;
            };
            let Some(conn) = &mut output.port else {
                continue;
            };
            visitor(&device.model, OutputPort { conn });
        }
    }
}

/// A control "slot" that can have a MIDI device connected to it.
pub struct DeviceSlot<D: 'static + Clone> {
    /// The name of this slot. Must be unique in the context of a single manager.
    name: String,
    /// The physical device wired up to this slot. If None, this slot is empty.
    device: Option<DeviceState<D>>,
}

struct DeviceState<D: 'static + Clone> {
    model: D,
    /// The input wired up to this device. If None, no input has been assigned.
    input: Option<DeviceInput<D>>,
    /// The output wired up to this device. If None, no output has been assigned.
    output: Option<DeviceOutput>,
}

impl<D: 'static + Clone + Send> DeviceState<D> {
    /// Connect the provided device ID to this input.
    ///
    /// Any existing device will be replaced.
    pub fn connect_input<F>(&mut self, id: DeviceId, handle_msg: F) -> Result<()>
    where
        F: Fn(Event, &D) + Send + 'static,
    {
        let input = MidiInput::new(&id.0)?;
        let Some(port) = input.ports().into_iter().find(|p| DeviceId(p.id()) == id) else {
            bail!("no MIDI input found with {id:?}");
        };
        let name = input
            .port_name(&port)
            .with_context(|| format!("unable to get port name for {id:?}"))?;
        let handler_name = name.clone();
        let conn = input
            .connect(
                &port,
                &name,
                move |_timestamp, msg: &[u8], model| {
                    let event = match Event::parse(msg) {
                        Ok(event) => event,
                        Err(err) => {
                            debug!("Ignoring midi input event on {handler_name}: {err:?}.");
                            return;
                        }
                    };
                    handle_msg(event, model);
                },
                self.model.clone(),
            )
            .with_context(|| name.clone())?;
        self.input = Some(DeviceInput {
            id,
            name,
            port: Some(conn),
        });
        Ok(())
    }

    /// Connect the provided device ID to this output.
    ///
    /// Any existing device will be replaced.
    pub fn connect_output<F>(&mut self, id: DeviceId) -> Result<()> {
        let output = MidiOutput::new(&id.0)?;
        let Some(port) = output.ports().into_iter().find(|p| DeviceId(p.id()) == id) else {
            bail!("no MIDI output found with {id:?}");
        };
        let name = output
            .port_name(&port)
            .with_context(|| format!("unable to get port name for {id:?}"))?;
        let conn = output.connect(&port, &name).with_context(|| name.clone())?;
        self.output = Some(DeviceOutput {
            id,
            name,
            port: Some(conn),
        });
        Ok(())
    }
}

struct DeviceInput<D: 'static + Clone> {
    id: DeviceId,
    name: String,
    /// If None, the device is disconnected.
    port: Option<MidiInputConnection<D>>,
}

struct DeviceOutput {
    id: DeviceId,
    name: String,
    /// If None, the device is disconnected.
    port: Option<MidiOutputConnection>,
}

/// Helper wrapper around a port to provide a more convenient interface.
pub struct OutputPort<'a> {
    conn: &'a mut MidiOutputConnection,
}

impl<'a> OutputPort<'a> {
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
