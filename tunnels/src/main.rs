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

use show::{setup_mutli_channel_test, Show};
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::{error::Error, time::Duration};

fn main() -> Result<(), Box<dyn Error>> {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())?;
    let mut show = Show::new(Vec::new())?;
    show.test_mode(Box::new(setup_mutli_channel_test));
    show.run(Duration::from_micros(16667))
}
