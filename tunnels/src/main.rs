mod animation;
mod beam;
mod beam_store;
mod clock;
mod device;
mod look;
mod master_ui;
mod midi;
mod midi_controls;
mod mixer;
mod numbers;
mod send;
mod show;
mod timesync;
mod tunnel;
mod waveforms;

use device::Device;
use log::info;
use midi::{list_ports, DeviceSpec};
use show::{setup_multi_channel_test, Show};
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::{error::Error, time::Duration};

fn main() -> Result<(), Box<dyn Error>> {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())?;
    let (inputs, outputs) = list_ports()?;
    info!("Inputs:\n{}\n\nOutputs:\n{}", inputs, outputs);
    let mut show = Show::new(vec![DeviceSpec {
        device: Device::TouchOsc,
        input_port_name: "Network Network Session 1".to_string(),
        output_port_name: "Network Network Session 1".to_string(),
    }])?;
    //show.test_mode(Box::new(setup_multi_channel_test));
    show.run(Duration::from_micros(16667))
}
