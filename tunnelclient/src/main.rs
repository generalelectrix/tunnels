mod constants {
    use std::f64::consts::PI;
    pub const TWOPI: f64 = 2.0 * PI;
}

mod config;
mod draw;
mod remote;
mod show;

use crate::config::ClientConfig;
use crate::remote::{administrate, run_remote};
use crate::show::Show;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::env;
use tunnels_lib::RunFlag;
use zmq::Context;

fn main() {
    // Check if running in remote mode.
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'remote' to run in remote mode, \
        'admin' to run the client administrator,
         or the integer virtual video channel to listen to.",
    );

    let ctx = Context::new();

    if first_arg == "remote" {
        init_logger(LevelFilter::Info);
        run_remote(ctx);
    } else if first_arg == "admin" {
        init_logger(LevelFilter::Info);
        administrate();
    } else {
        let video_channel: u64 = first_arg
            .parse()
            .expect("Video channel must be a positive integer.");

        let config_path = env::args().nth(2).expect("No config path arg provided.");

        let cfg = ClientConfig::load(video_channel, &config_path).expect("Failed to load config");
        init_logger(if cfg.log_level_debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        });

        let mut show = Show::new(cfg, ctx, RunFlag::default()).expect("Failed to initialize show");

        show.run();
    }
}

fn init_logger(level: LevelFilter) {
    SimpleLogger::init(level, LogConfig::default()).expect("Could not configure logger.");
}
