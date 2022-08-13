use std::error::Error;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use crate::master_ui::EmitStateChange;
use crate::midi_controls::Dispatcher as MidiDispatcher;
use crate::show::{ControlMessage, StateChange};
use crate::{
    midi::{DeviceSpec, Event as MidiEvent},
    midi_controls::Device,
};

/// Top-level enum for the types of control messages the show can receive.
pub enum ControlEvent {
    Midi((Device, MidiEvent)),
    Osc((Device, OscEvent)),
}

pub struct OscEvent;

pub struct Dispatcher {
    midi_dispatcher: MidiDispatcher,
    recv: Receiver<ControlEvent>,
}

impl Dispatcher {
    /// Instantiate the master control dispatcher.
    pub fn new(midi_devices: Vec<DeviceSpec>) -> Result<Self, Box<dyn Error>> {
        let (send, recv) = channel();

        Ok(Self {
            midi_dispatcher: MidiDispatcher::new(midi_devices, send.clone())?,
            recv,
        })
    }

    pub fn receive(&self, timeout: Duration) -> Option<ControlMessage> {
        self.recv.recv_timeout(timeout).ok().and_then(|event| {
            use ControlEvent::*;
            match event {
                Midi((device, event)) => self
                    .midi_dispatcher
                    .map_event_to_show_control(device, event),
                Osc(_) => {
                    // TODO
                    None
                }
            }
        })
    }
}

impl EmitStateChange for Dispatcher {
    /// Map application state changes into UI update messages.
    fn emit(&mut self, sc: StateChange) {
        self.midi_dispatcher.emit(sc);
    }
}
