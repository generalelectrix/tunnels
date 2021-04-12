use log;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver};

/// How many virtual video channels should we send?
const N_VIDEO_CHANNELS: i32 = 8;

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

#[derive(Clone, Debug)]
pub struct Show {
    config: Config,
}
