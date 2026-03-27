use anyhow::{Context as _, Result};
use midi_harness::DeviceChange;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::master_ui::EmitStateChange;
use crate::midi_controls::Dispatcher as MidiDispatcher;
use crate::osc;
use crate::show::{ControlMessage, StateChange};
use crate::{
    midi::{DeviceSpec as MidiDeviceSpec, Event as MidiEvent},
    midi_controls::Device as MidiDevice,
    osc::{Device as OscDevice, DeviceSpec as OscDeviceSpec},
};
use anyhow::bail;
use rosc::OscMessage;

/// The result of processing a MetaCommand.
pub type CommandResponse = Result<(), String>;

/// Commands for show-level meta-control: configuration changes,
/// system actions, and lifecycle events.
#[derive(Debug)]
pub enum MetaCommand {
    RefreshUI,
    AddMidiDevice(MidiDeviceSpec<MidiDevice>),
    ClearMidiDevice { slot_name: String },
    ConnectMidiPort {
        slot_name: String,
        device_id: midi_harness::DeviceId,
        kind: midi_harness::DeviceKind,
    },
    SetAudioDevice(Option<String>),
}

/// A handle for sending commands to the show and waiting for responses.
#[derive(Clone)]
pub struct CommandClient {
    send: Sender<ControlEvent>,
}

impl CommandClient {
    pub fn new(send: Sender<ControlEvent>) -> Self {
        Self { send }
    }

    /// Send a command and block until the show responds.
    pub fn send_command(&self, cmd: MetaCommand) -> Result<()> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.send
            .send(ControlEvent::Meta(cmd, Some(reply_tx)))
            .map_err(|_| anyhow::anyhow!("show control channel disconnected"))?;
        reply_rx
            .recv()
            .context("show did not send a response")?
            .map_err(|e| anyhow::anyhow!(e))
    }
}

/// Top-level enum for the types of control messages the show can receive.
pub enum ControlEvent {
    MidiDevice(DeviceChange),
    Midi((MidiDevice, MidiEvent)),
    Osc((OscDevice, OscMessage)),
    Meta(MetaCommand, Option<Sender<CommandResponse>>),
}

/// What the show receives from the dispatcher after event processing.
pub enum ReceivedEvent {
    /// A real-time show control message (MIDI knob, OSC, etc.).
    Control(ControlMessage),
    /// A meta-command with an optional reply channel.
    Meta(MetaCommand, Option<Sender<CommandResponse>>),
}

pub struct Dispatcher {
    midi_dispatcher: MidiDispatcher,
    recv: Receiver<ControlEvent>,
    send: Sender<ControlEvent>,
}

impl Dispatcher {
    /// Instantiate the master control dispatcher.
    pub fn new(
        midi_devices: Vec<MidiDeviceSpec<MidiDevice>>,
        osc_devices: Vec<OscDeviceSpec>,
        send: Sender<ControlEvent>,
        recv: Receiver<ControlEvent>,
    ) -> Result<Self> {
        for osc_device in osc_devices {
            osc::listen(osc_device, send.clone())?;
        }

        let dispatcher_send = send.clone();
        Ok(Self {
            midi_dispatcher: MidiDispatcher::new(midi_devices, send)?,
            recv,
            send: dispatcher_send,
        })
    }

    pub fn receive(&mut self, timeout: Duration) -> Result<Option<ReceivedEvent>> {
        let event = match self.recv.recv_timeout(timeout) {
            Ok(e) => e,
            Err(RecvTimeoutError::Timeout) => {
                return Ok(None);
            }
            Err(RecvTimeoutError::Disconnected) => {
                bail!("Control event channel is disconnected!");
            }
        };
        use ControlEvent::*;
        match event {
            MidiDevice(event) => {
                let needs_ui_refresh = self.midi_dispatcher.handle_device_change(event)?;
                if needs_ui_refresh {
                    // Fire-and-forget — no reply needed for device-reconnect refresh.
                    let _ = self.send.send(ControlEvent::Meta(MetaCommand::RefreshUI, None));
                }
                Ok(None)
            }
            Midi((device, event)) => Ok(self
                .midi_dispatcher
                .map_event_to_show_control(device, event)
                .map(ReceivedEvent::Control)),
            Osc((device, event)) => Ok(osc::map_event_to_show_control(device, event)?
                .map(ReceivedEvent::Control)),
            Meta(cmd, reply) => Ok(Some(ReceivedEvent::Meta(cmd, reply))),
        }
    }

    pub fn add_midi_device(&mut self, spec: MidiDeviceSpec<MidiDevice>) -> Result<()> {
        self.midi_dispatcher.add_midi_device(spec)
    }

    pub fn clear_midi_device(&mut self, slot_name: &str) -> Result<()> {
        self.midi_dispatcher.clear_midi_device(slot_name)
    }

    pub fn connect_midi_port(
        &mut self,
        slot_name: &str,
        device_id: midi_harness::DeviceId,
        kind: midi_harness::DeviceKind,
    ) -> Result<()> {
        self.midi_dispatcher.connect_midi_port(slot_name, device_id, kind)
    }

    pub fn midi_slot_statuses(&self) -> Vec<midi_harness::SlotStatus> {
        self.midi_dispatcher.midi_slot_statuses()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod mock {
    use super::*;

    /// Create a CommandClient that auto-responds Ok(()) to every command.
    pub fn auto_respond_client() -> CommandClient {
        let (send, recv) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(ControlEvent::Meta(_, Some(reply))) = recv.recv() {
                let _ = reply.send(Ok(()));
            }
        });
        CommandClient::new(send)
    }
}

impl EmitStateChange for Dispatcher {
    /// Map application state changes into UI update messages.
    fn emit(&mut self, sc: StateChange) {
        self.midi_dispatcher.emit(sc);
        // FIXME: need to borrow state change messages instead of moving them
        // if we want state changes to fan-out to different control types.
        // self.osc_dispatcher.emit(sc);
    }
}
