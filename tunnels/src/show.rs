use log;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver};

use crate::{
    beam_matrix_minder::BeamMatrixMinder, clock::ClockBank, device::Device, midi::Manager,
    mixer::Mixer, tunnel,
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
    midi_manager: Manager,
    mixer: Mixer,
    clocks: ClockBank,
    beam_matrix: BeamMatrixMinder,
}

pub enum ControlMessage {
    Tunnel(tunnel::ControlMessage),
}

pub enum StateChange {
    Tunnel(tunnel::StateChange),
}
