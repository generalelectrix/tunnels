mod show;

use crate::show::Show;
use client_lib::config::ClientConfig;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::env;
use std::process::ExitCode;
use tunnels_lib::RunFlag;

fn main() -> ExitCode {
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'monitor' to run a local monitor (config via stdin), \
        or the integer virtual video channel to listen to.",
    );

    if first_arg == "monitor" {
        let cfg: ClientConfig = match rmp_serde::from_read(std::io::stdin()) {
            Ok(cfg) => cfg,
            Err(e) => {
                println!("ERROR: failed to deserialize config: {e}");
                return ExitCode::FAILURE;
            }
        };
        match Show::new(cfg, RunFlag::default()) {
            Ok(mut show) => {
                println!("OK");
                show.run();
            }
            Err(e) => {
                println!("ERROR: {e}");
                return ExitCode::FAILURE;
            }
        }
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

        let mut show = Show::new(cfg, RunFlag::default()).expect("Failed to initialize show");

        show.run();
    }

    ExitCode::SUCCESS
}

fn init_logger(level: LevelFilter) {
    SimpleLogger::init(level, LogConfig::default()).expect("Could not configure logger.");
}
