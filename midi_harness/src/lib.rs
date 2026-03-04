//! MIDI device management to plug into an event-driven environment.
mod device_change;
pub mod event;

use anyhow::{Context, Result, bail};
pub use device_change::{DeviceId, initialize};
use log::debug;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, SendError};

use crate::event::{Event, EventType};

pub struct DeviceManager<D, H>
where
    D: InitMidiDevice,
    H: MidiHandler<D>,
{
    slots: Vec<DeviceSlot<D>>,
    /// Handler that will be called by a MIDI input on message arrival.
    ///
    /// Accepts the MIDI Event, and will also be handed your device model to be
    /// used to interpret the message. This callback should do something with
    /// the result, such as push it onto a channel.
    handler: H,
}

/// Something that handles a MIDI event using a provided device model for interpretation.
pub trait MidiHandler<D>: Clone + Send + 'static {
    /// Handle the event.
    fn handle(&self, event: Event, device: &D);
}

impl<D, H> DeviceManager<D, H>
where
    D: InitMidiDevice,
    H: MidiHandler<D>,
{
    /// Initialize the manager.
    pub fn new(handler: H) -> Self {
        Self {
            slots: vec![],
            handler,
        }
    }

    /// Add a new slot. Return an error if we already have a slot with this name.
    pub fn add_slot(&mut self, name: String, model: D) -> Result<()> {
        if self.slots.iter().any(|s| s.name == name) {
            bail!("refusing to add a MIDI device slot with duplicate name \"{name}\"");
        }
        self.slots.push(DeviceSlot {
            name,
            model,
            input: None,
            output: None,
        });
        Ok(())
    }

    /// Call the provided closure on each connected output.
    /// The attached model and the MIDI output are provided.
    pub fn visit_outputs(&mut self, visitor: impl Fn(&D, OutputPort)) {
        for slot in &mut self.slots {
            let Some(output) = &mut slot.output else {
                continue;
            };
            let Some(conn) = &mut output.port else {
                continue;
            };
            visitor(
                &slot.model,
                OutputPort {
                    conn,
                    name: &output.name,
                },
            );
        }
    }

    /// Connect the provided device ID to the input in the named slot.
    pub fn connect_input(&mut self, slot: &str, id: DeviceId) -> Result<()> {
        let Some(slot) = self.slots.iter_mut().find(|s| s.name == slot) else {
            bail!("unknown device slot {slot}");
        };
        slot.connect_input(id, self.handler.clone())
    }

    /// Connect the provided device ID to the output in the named slot.
    pub fn connect_output(&mut self, slot: &str, id: DeviceId) -> Result<()> {
        let Some(slot) = self.slots.iter_mut().find(|s| s.name == slot) else {
            bail!("unknown device slot {slot}");
        };
        slot.connect_output(id)
    }

    /// Set the specified device as disconnected, if it is connected.
    pub fn mark_disconnected(&mut self, id: &DeviceId) {
        for slot in &mut self.slots {
            slot.mark_disconnected(id);
        }
    }

    /// If any slot is connected to the provided device ID and is disconnected,
    /// try to reconnect. Return true if we successfully reconnected a device.
    pub fn try_reconnect(&mut self, id: &DeviceId) -> Result<bool> {
        for slot in &mut self.slots {
            if slot.try_reconnect(id, &self.handler)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// A control "slot" that can have a MIDI device connected to it.
pub struct DeviceSlot<D: InitMidiDevice> {
    /// The name of this slot. Must be unique in the context of a single manager.
    name: String,
    model: D,
    /// The input wired up to this device. If None, no input has been assigned.
    input: Option<DeviceInput<D>>,
    /// The output wired up to this device. If None, no output has been assigned.
    output: Option<DeviceOutput>,
}

impl<D: InitMidiDevice + Send> DeviceSlot<D> {
    /// Connect the provided device ID to this input.
    ///
    /// Any existing device will be replaced.
    pub fn connect_input(&mut self, id: DeviceId, handler: impl MidiHandler<D>) -> Result<()> {
        self.input = None;
        let mut input = DeviceInput {
            id,
            name: String::new(),
            port: None,
        };
        input.connect(self.model.clone(), handler)?;
        self.input = Some(input);
        Ok(())
    }

    /// Connect the provided device ID to this output.
    ///
    /// Any existing device will be replaced.
    pub fn connect_output(&mut self, id: DeviceId) -> Result<()> {
        self.output = None;
        let mut output = DeviceOutput {
            id,
            name: String::new(),
            port: None,
        };
        output.connect(&self.model)?;
        self.output = Some(output);
        Ok(())
    }

    /// Set the specified device as disconnected if it is attached to this slot.
    pub fn mark_disconnected(&mut self, id: &DeviceId) {
        if let Some(input) = self.input.as_mut()
            && &input.id == id
        {
            input.port = None;
        }
        if let Some(output) = self.output.as_mut()
            && &output.id == id
        {
            output.port = None;
        }
    }

    /// Potentially attempt reconnect.
    pub fn try_reconnect(&mut self, id: &DeviceId, handler: &impl MidiHandler<D>) -> Result<bool> {
        if let Some(input) = self.input.as_mut()
            && input.id == *id
            && input.port.is_none()
        {
            input.connect(self.model.clone(), handler.clone())?;
            return Ok(true);
        }
        if let Some(output) = self.output.as_mut()
            && output.id == *id
            && output.port.is_none()
        {
            output.connect(&self.model)?;
            return Ok(true);
        }
        Ok(false)
    }
}

struct DeviceInput<D: InitMidiDevice> {
    id: DeviceId,
    name: String,
    /// If None, the device is disconnected.
    port: Option<MidiInputConnection<D>>,
}

impl<D: InitMidiDevice> DeviceInput<D> {
    /// Connect the currently-assigned device.
    ///
    /// Any existing device will be replaced.
    pub fn connect(&mut self, model: D, handler: impl MidiHandler<D>) -> Result<()> {
        let input = MidiInput::new(&self.id.0)?;
        let Some(port) = input
            .ports()
            .into_iter()
            .find(|p| DeviceId(p.id()) == self.id)
        else {
            bail!("no MIDI input found with {:?}", self.id);
        };
        let name = input
            .port_name(&port)
            .with_context(|| format!("unable to get port name for {:?}", self.id))?;
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
                    handler.handle(event, model);
                },
                model,
            )
            .with_context(|| name.clone())?;
        self.port = Some(conn);
        self.name = name;
        Ok(())
    }
}

struct DeviceOutput {
    id: DeviceId,
    name: String,
    /// If None, the device is disconnected.
    port: Option<MidiOutputConnection>,
}

impl DeviceOutput {
    /// Connect the currently-assigned device.
    ///
    /// Any existing device will be replaced.
    ///
    /// The device will be initialized.
    pub fn connect<D: InitMidiDevice>(&mut self, device: &D) -> Result<()> {
        let output = MidiOutput::new(&self.id.0)?;
        let Some(port) = output
            .ports()
            .into_iter()
            .find(|p| DeviceId(p.id()) == self.id)
        else {
            bail!("no MIDI output found with {:?}", self.id);
        };
        let name = output
            .port_name(&port)
            .with_context(|| format!("unable to get port name for {:?}", self.id))?;
        let mut conn = output.connect(&port, &name).with_context(|| name.clone())?;
        device.init_midi(OutputPort {
            conn: &mut conn,
            name: &self.name,
        })?;
        self.port = Some(conn);
        Ok(())
    }
}

/// Helper wrapper around a port to provide a more convenient interface.
pub struct OutputPort<'a> {
    conn: &'a mut MidiOutputConnection,
    name: &'a String,
}

impl<'a> OutputPort<'a> {
    /// Send a MIDI event.
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

    /// Send a raw byte buffer to this MIDI device.
    pub fn send_raw(&mut self, msg: &[u8]) -> Result<(), SendError> {
        self.conn.send(msg)
    }

    /// Get the name of the device associated with this port.
    pub fn name(&self) -> &str {
        self.name
    }
}

pub trait InitMidiDevice: Sized + Send + Clone + 'static {
    /// Perform device-specific midi initialization.
    #[allow(unused)]
    fn init_midi(&self, out: OutputPort<'_>) -> Result<()> {
        Ok(())
    }
}
