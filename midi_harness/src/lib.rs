//! MIDI device management to plug into an event-driven environment.
mod device_change;
pub mod event;
mod select;
pub use select::*;

use anyhow::{Context, Result, anyhow, bail};
pub use device_change::{
    DeviceChange, DeviceId, DeviceKind, HandleDeviceChange, install_midi_device_change_handler,
};
use log::{debug, error, info};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection, SendError};

use crate::event::{Event, EventType};

/// Connection state for one direction (input or output) of a MIDI device slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortStatus {
    /// No device has been assigned to this port direction.
    Unassigned,
    /// A device is assigned but not currently connected (e.g. unplugged).
    Disconnected {
        /// Unique system identifier for the port.
        id: DeviceId,
        /// Human-readable name of the port from last connection.
        name: String,
    },
    /// A device is assigned and actively connected.
    Connected {
        /// Unique system identifier for the port.
        id: DeviceId,
        /// Human-readable name of the port.
        name: String,
    },
}

/// A serialization-friendly snapshot of a single MIDI device slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotStatus {
    /// The slot name (e.g. "Submaster Wing 1", "Clock Wing").
    pub name: String,
    /// String representation of the device type expected by this slot.
    pub model: String,
    /// State of the MIDI input port.
    pub input: PortStatus,
    /// State of the MIDI output port.
    pub output: PortStatus,
}

fn port_status(id: &DeviceId, name: &str, port_is_some: bool) -> PortStatus {
    if port_is_some {
        PortStatus::Connected {
            id: id.clone(),
            name: name.to_string(),
        }
    } else {
        PortStatus::Disconnected {
            id: id.clone(),
            name: name.to_string(),
        }
    }
}

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
///
/// Also provides handling for device change notifications.
pub trait MidiHandler<D>: Clone + Send + 'static {
    /// Handle the event.
    fn handle(&self, event: Event, device: &D);
}

impl<D, H> DeviceManager<D, H>
where
    D: InitMidiDevice,
    H: MidiHandler<D> + HandleDeviceChange,
{
    /// Initialize the manager, and set up device change notification using the
    /// same provided handler.
    pub fn with_device_changes(handler: H) -> Result<Self> {
        install_midi_device_change_handler(handler.clone())?;
        Ok(Self::new(handler))
    }
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

    /// Helper method to wire up a device in full, from pieces.
    /// The input device ID will be used as the slot name.
    /// This is a shim to make it easy to use legacy up-front configuration methods.
    pub fn add_from_spec(
        &mut self,
        device: D,
        input_id: DeviceId,
        output_id: DeviceId,
    ) -> Result<()> {
        let slot_name = input_id.0.clone();
        self.add_slot(slot_name.clone(), device)?;
        self.connect_input(&slot_name, input_id)?;
        self.connect_output(&slot_name, output_id)?;
        Ok(())
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

    /// Remove a slot by name.
    pub fn remove_slot(&mut self, name: &str) -> Result<()> {
        let index = self
            .slots
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| anyhow!("no MIDI device slot named \"{name}\""))?;
        self.slots.remove(index);
        Ok(())
    }

    /// Call the provided closure on each connected output.
    /// The attached model and the MIDI output are provided.
    pub fn visit_outputs(&mut self, visitor: impl Fn(&D, &mut OutputPort)) {
        for slot in &mut self.slots {
            let Some(output) = &mut slot.output else {
                continue;
            };
            let Some(conn) = &mut output.port else {
                continue;
            };
            visitor(
                &slot.model,
                &mut OutputPort {
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

    /// Clear the device assignment from the named slot.
    ///
    /// The slot remains but with no input or output assigned.
    /// Any active connections are dropped. After clearing, `try_reconnect`
    /// will not match this slot.
    pub fn clear_slot(&mut self, slot_name: &str) -> Result<()> {
        let Some(slot) = self.slots.iter_mut().find(|s| s.name == slot_name) else {
            bail!("unknown device slot {slot_name}");
        };
        slot.input = None;
        slot.output = None;
        Ok(())
    }

    /// Return the names of all slots.
    pub fn slot_names(&self) -> Vec<String> {
        self.slots.iter().map(|s| s.name.clone()).collect()
    }

    /// Return a snapshot of the status of every slot.
    pub fn slot_statuses(&self) -> Vec<SlotStatus>
    where
        D: std::fmt::Display,
    {
        self.slots
            .iter()
            .map(|slot| SlotStatus {
                name: slot.name.clone(),
                model: slot.model.to_string(),
                input: slot
                    .input
                    .as_ref()
                    .map(|i| port_status(&i.id, &i.name, i.port.is_some()))
                    .unwrap_or(PortStatus::Unassigned),
                output: slot
                    .output
                    .as_ref()
                    .map(|o| port_status(&o.id, &o.name, o.port.is_some()))
                    .unwrap_or(PortStatus::Unassigned),
            })
            .collect()
    }

    /// Connect the provided device ID to the output in the named slot.
    pub fn connect_output(&mut self, slot: &str, id: DeviceId) -> Result<()> {
        let Some(slot) = self.slots.iter_mut().find(|s| s.name == slot) else {
            bail!("unknown device slot {slot}");
        };
        slot.connect_output(id)
    }

    /// Set the specified device as disconnected, if it is connected.
    ///
    /// Return true if an active device was disconnected.
    fn mark_disconnected(&mut self, id: &DeviceId) -> bool {
        let mut disconnected = false;
        for slot in &mut self.slots {
            disconnected |= slot.mark_disconnected(id);
        }
        disconnected
    }

    /// If any slot is connected to the provided device ID and is disconnected,
    /// try to reconnect. Return true if we successfully reconnected a device.
    fn try_reconnect(&mut self, id: &DeviceId) -> Result<bool> {
        for slot in &mut self.slots {
            if slot.try_reconnect(id, &self.handler)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Handle a device appearing or disappearing.
    ///
    /// Return Some(kind) if we reconnected an input or output.
    pub fn handle_device_change(&mut self, change: DeviceChange) -> Result<Option<DeviceKind>> {
        match change {
            DeviceChange::Connected { id, name, kind } => {
                let reconnected = self.try_reconnect(&id)?;
                Ok(if reconnected {
                    info!("successfully reconnected MIDI device {name}");
                    Some(kind)
                } else {
                    None
                })
            }
            DeviceChange::Disconnected(id) => {
                let disconnected = self.mark_disconnected(&id);
                if disconnected {
                    // FIXME: this would be a lot more useful with a name
                    error!("MIDI device {id:?} disconnected.");
                }
                Ok(None)
            }
        }
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
    pub fn mark_disconnected(&mut self, id: &DeviceId) -> bool {
        let mut disconnected = false;
        if let Some(input) = self.input.as_mut()
            && &input.id == id
        {
            input.port = None;
            disconnected = true;
        }
        if let Some(output) = self.output.as_mut()
            && &output.id == id
        {
            output.port = None;
            disconnected = true;
        }
        disconnected
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
            .map_err(|err| anyhow!("failed to open MIDI input {name}: {err}"))?;
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
        let mut conn = output
            .connect(&port, &name)
            .map_err(|err| anyhow!("failed to open MIDI output {name}: {err}"))?;
        device.init_midi(&mut OutputPort {
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

impl<'a> Output for OutputPort<'a> {
    /// Send a MIDI event.
    fn send(&mut self, event: Event) -> Result<(), SendError> {
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
    fn send_raw(&mut self, msg: &[u8]) -> Result<(), SendError> {
        self.conn.send(msg)
    }

    /// Get the name of the device associated with this port.
    fn name(&self) -> &str {
        self.name
    }
}

pub trait InitMidiDevice: Sized + Send + Clone + 'static {
    /// Perform device-specific midi initialization.
    #[allow(unused)]
    fn init_midi(&self, out: &mut dyn Output) -> Result<()> {
        Ok(())
    }
}

/// Behaviors provided by a generic MIDI output.
pub trait Output {
    /// Send a MIDI event.
    fn send(&mut self, event: Event) -> Result<(), SendError>;

    /// Send a raw byte buffer to this MIDI device.
    fn send_raw(&mut self, msg: &[u8]) -> Result<(), SendError>;

    /// Get the name of the device associated with this port.
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestDevice;
    impl InitMidiDevice for TestDevice {}

    impl std::fmt::Display for TestDevice {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("TestDevice")
        }
    }

    #[derive(Clone)]
    struct TestHandler;
    impl MidiHandler<TestDevice> for TestHandler {
        fn handle(&self, _event: Event, _device: &TestDevice) {}
    }

    #[test]
    fn remove_slot_success() {
        let mut mgr = DeviceManager::new(TestHandler);
        mgr.add_slot("slot-a".into(), TestDevice).unwrap();
        mgr.add_slot("slot-b".into(), TestDevice).unwrap();
        assert_eq!(mgr.slot_names().len(), 2);

        mgr.remove_slot("slot-a").unwrap();
        assert_eq!(mgr.slot_names(), vec!["slot-b".to_string()]);
    }

    #[test]
    fn remove_slot_unknown_name() {
        let mut mgr = DeviceManager::new(TestHandler);
        let err = mgr.remove_slot("nope").unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn slot_statuses_returns_correct_data() {
        let mut mgr = DeviceManager::new(TestHandler);
        mgr.add_slot("Wing 1".into(), TestDevice).unwrap();
        mgr.add_slot("Clock".into(), TestDevice).unwrap();

        let statuses = mgr.slot_statuses();
        assert_eq!(statuses.len(), 2);

        assert_eq!(statuses[0].name, "Wing 1");
        assert_eq!(statuses[0].model, "TestDevice");
        assert_eq!(statuses[0].input, PortStatus::Unassigned);
        assert_eq!(statuses[0].output, PortStatus::Unassigned);

        assert_eq!(statuses[1].name, "Clock");
        assert_eq!(statuses[1].model, "TestDevice");
        assert_eq!(statuses[1].input, PortStatus::Unassigned);
        assert_eq!(statuses[1].output, PortStatus::Unassigned);
    }
}
