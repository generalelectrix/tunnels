mod artnet_node;
mod show;

use crate::show::Show;
use client_lib::config::ClientConfig;
use simplelog::{Config as LogConfig, LevelFilter, WriteLogger};
use std::env;
use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'licenses' to print the figure attribution, \
        'monitor' to run a local monitor (config via stdin), \
        or the integer virtual video channel to listen to.",
    );

    if first_arg == "licenses" {
        print_licenses();
    } else if first_arg == "monitor" {
        let cfg: ClientConfig = match read_config(std::io::stdin()) {
            Ok(cfg) => cfg,
            Err(e) => {
                println!("ERROR: failed to deserialize config: {e}");
                return ExitCode::FAILURE;
            }
        };
        init_logger(&cfg);
        match Show::new(cfg) {
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

        let mut show = Show::new(cfg).expect("Failed to initialize show");

        show.run();
    }

    ExitCode::SUCCESS
}

/// Print where the baked figures came from and the terms they carry.
///
/// The figures are derived from a font under the SIL Open Font License and
/// from a Wikimedia drawing under CC BY-SA, both of which require their notice
/// and licence to travel with what is distributed. This binary is what gets
/// distributed — the bootstrapper pushes it to a machine during a show — so
/// the notice is compiled into it and this is how it is read back out.
fn print_licenses() {
    println!("{}", tunnels_shapes::ATTRIBUTION);
    println!("{}", tunnels_shapes::OPEN_FONT_LICENSE);
    println!(
        "Per-figure sources and licences, for all {} figures:\n{}",
        tunnels_shapes::count(),
        tunnels_shapes::CREDITS
    );
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
