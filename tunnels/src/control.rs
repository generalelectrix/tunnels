use anyhow::Result;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::master_ui::EmitStateChange;
use crate::midi_controls::Dispatcher as MidiDispatcher;
use crate::show::{ControlMessage, StateChange};
use crate::{
    midi::{DeviceSpec as MidiDeviceSpec, Event as MidiEvent},
    midi_controls::Device as MidiDevice,
    osc::{Device as OscDevice, DeviceSpec as OscDeviceSpec, Dispatcher as OscDispatcher},
};
use anyhow::bail;
use rosc::OscMessage;

/// Top-level enum for the types of control messages the show can receive.
pub enum ControlEvent {
    Midi((MidiDevice, MidiEvent)),
    Osc((OscDevice, OscMessage)),
}

pub struct Dispatcher {
    midi_dispatcher: MidiDispatcher,
    osc_dispatcher: OscDispatcher,
    recv: Receiver<ControlEvent>,
}

impl Dispatcher {
    /// Instantiate the master control dispatcher.
    pub fn new(midi_devices: Vec<MidiDeviceSpec>, osc_devices: Vec<OscDeviceSpec>) -> Result<Self> {
        let (send, recv) = channel();

        Ok(Self {
            midi_dispatcher: MidiDispatcher::new(midi_devices, send.clone())?,
            osc_dispatcher: OscDispatcher::new(osc_devices, send)?,
            recv,
        })
    }

    pub fn receive(&self, timeout: Duration) -> Result<Option<ControlMessage>> {
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
            Midi((device, event)) => Ok(self
                .midi_dispatcher
                .map_event_to_show_control(device, event)),
            Osc((device, event)) => self.osc_dispatcher.map_event_to_show_control(device, event),
        }
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
