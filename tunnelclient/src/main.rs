mod remote;
mod show;

use crate::remote::{administrate, run_remote};
use crate::show::Show;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::env;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;
use tunnelclient::bootstrap_controller::BootstrapController;
use client_lib::config::ClientConfig;
use tunnels_lib::RunFlag;
use zmq::Context;

fn main() -> ExitCode {
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'remote' to run in remote mode, \
        'admin' to run the client administrator, \
        'monitor' to run a local monitor (config via stdin), \
        or the integer virtual video channel to listen to.",
    );

    let ctx = Context::new();

    if first_arg == tunnelclient::ARG_SELF_TEST {
        // Lightweight health check — no window, no graphics.
        // Verify ZMQ context + socket creation.
        let _ = ctx.socket(zmq::REP).expect("ZMQ socket creation failed");

        // Verify DNS-SD is available.
        let stop = zero_configure::bare::register_service("selftest", 0)
            .expect("DNS-SD registration failed");
        stop();

        println!("self-test passed");
        std::process::exit(0);
    } else if first_arg == "push" {
        init_logger(LevelFilter::Info);
        push_to_first_bootstrapper(ctx);
    } else if first_arg == tunnelclient::ARG_REMOTE {
        init_logger(LevelFilter::Info);
        run_remote(ctx);
    } else if first_arg == "admin" {
        init_logger(LevelFilter::Info);
        administrate();
    } else if first_arg == "monitor" {
        let cfg: ClientConfig = match rmp_serde::from_read(std::io::stdin()) {
            Ok(cfg) => cfg,
            Err(e) => {
                println!("ERROR: failed to deserialize config: {e}");
                return ExitCode::FAILURE;
            }
        };
        match Show::new(cfg, ctx, RunFlag::default()) {
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

        let mut show = Show::new(cfg, ctx, RunFlag::default()).expect("Failed to initialize show");

        show.run();
    }

    ExitCode::SUCCESS
}

fn init_logger(level: LevelFilter) {
    SimpleLogger::init(level, LogConfig::default()).expect("Could not configure logger.");
}

/// Browse for the first available bootstrapper and push ourselves to it.
fn push_to_first_bootstrapper(ctx: Context) {
    let binary_path = env::current_exe().expect("Could not determine own binary path");
    let controller = BootstrapController::new(ctx);

    println!("Browsing for bootstrappers...");

    let target = loop {
        let targets = controller.list();
        if let Some(name) = targets.first() {
            break name.clone();
        }
        thread::sleep(Duration::from_secs(1));
    };

    println!("Pushing {} to {target}...", binary_path.display());
    match controller.push_binary(
        &target,
        &binary_path,
        &[tunnelclient::ARG_SELF_TEST],
        &[tunnelclient::ARG_REMOTE],
    ) {
        Ok(msg) => println!("Success: {msg}"),
        Err(e) => println!("Push failed: {e}"),
    }
}
