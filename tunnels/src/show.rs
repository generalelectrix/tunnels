use log;
use serde::{Deserialize, Serialize};
use std::{
    sync::mpsc::{channel, Receiver},
    time::Duration,
};

use crate::{
    animation,
    beam_store::{self, BeamStore},
    clock::ClockBank,
    device::Device,
    midi::Manager,
    midi_controls::Dispatcher,
    mixer,
    mixer::Mixer,
    tunnel,
    ui::UI,
};

#[derive(Copy, Clone, Debug)]
pub enum TestMode {
    Stress,
    Rotation,
    Aliasing,
    MultiChannel,
}

#[derive(Clone, Debug)]
pub struct Config {
    use_midi: bool,
    midi_devices: Vec<String>,
    report_framerate: bool,
    log_level: log::Level,
    test_mode: Option<TestMode>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            use_midi: false,
            midi_devices: Vec::new(),
            report_framerate: false,
            log_level: log::Level::Debug,
            test_mode: None,
        }
    }
}

pub struct Show {
    config: Config,
    dispatcher: Dispatcher,
    ui: UI,
    mixer: Mixer,
    clocks: ClockBank,
    beam_matrix: BeamStore,
}

impl Show {
    fn process_input(&mut self) {
        if let Some(msg) = self.dispatcher.receive(Default::default()) {
            if let Some(control_message) = self.dispatcher.dispatch(msg.0, msg.1) {
                self.ui.handle_control_message(
                    control_message,
                    &mut self.mixer,
                    &mut self.dispatcher,
                )
            }
        }
    }
}

pub enum ControlMessage {
    Tunnel(tunnel::ControlMessage),
    Animation(animation::ControlMessage),
}

pub enum StateChange {
    Tunnel(tunnel::StateChange),
    Animation(animation::StateChange),
    Mixer(mixer::StateChange),
    //BeamStore(beam_store::StateChange),
}
