mod show;

use crate::show::Show;
use client_lib::config::ClientConfig;
use simplelog::{Config as LogConfig, LevelFilter, WriteLogger};
use std::env;
use std::io::Read;
use std::process::ExitCode;
use tunnels_lib::RunFlag;

fn main() -> ExitCode {
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'monitor' to run a local monitor (config via stdin), \
        or the integer virtual video channel to listen to.",
    );

    if first_arg == "monitor" {
        let cfg: ClientConfig = match read_config(std::io::stdin()) {
            Ok(cfg) => cfg,
            Err(e) => {
                println!("ERROR: failed to deserialize config: {e}");
                return ExitCode::FAILURE;
            }
        };
        init_logger(&cfg);
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
        init_logger(&cfg);

        let mut show = Show::new(cfg, RunFlag::default()).expect("Failed to initialize show");

        show.run();
    }

    ExitCode::SUCCESS
}

/// Read a client configuration from a stream that carries one and then ends.
///
/// The encoding is tagless, so a message has no end of its own: what bounds it
/// is the stream closing.
fn read_config(mut source: impl Read) -> anyhow::Result<ClientConfig> {
    let mut payload = Vec::new();
    source.read_to_end(&mut payload)?;
    Ok(postcard::from_bytes(&payload)?)
}

/// Send log records to stderr, at the level the configuration asks for.
///
/// stderr and not stdout: stdout carries a single startup status line, `OK` or
/// `ERROR: ...`, and a log record printed alongside it would be read as that
/// status. Nothing else is ever written to stdout.
fn init_logger(cfg: &ClientConfig) {
    let level = if cfg.log_level_debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    WriteLogger::init(level, LogConfig::default(), std::io::stderr())
        .expect("Could not configure logger.");
}
