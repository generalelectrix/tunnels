//! Enable remote control of a tunnel render slave over the network.
//! Advertise this slave for control over DNS-SD, handling requests on a 0mq socket.
//! Very basic control; every message received is a full configuration struct, and the receipt of
//! a message completely tears down an existing show and brings up a new one using the new
//! parameters.
//! Also provide the tools needed for simple remote administration.

use crate::show::Show;
use anyhow::Result;
use log::{error, info};
use rmp_serde::decode::from_read;
use std::io::{stdin, stdout, Write};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;
use tunnelclient::admin::{parse_resolution, Administrator};
use tunnelclient::config::ClientConfig;
use tunnelclient::draw::{Transform, TransformDirection};
use tunnels_lib::RunFlag;
use zero_configure::req_rep::run_service_req_rep;
use zmq::Context;

const SERVICE_NAME: &str = "tunnelclient";
const PORT: u16 = 15000;

// --- client remote control ---

/// Run this client as a remotely configurable service.
/// The show starts up in the main thread to ensure we don't end up with issues trying to pass
/// OpenGL resources between threads.
/// Spawn a second thread to run the remote service, passing configurations to run back across a
/// channel.
/// Panics if the remote service thread fails to spawn.
pub fn run_remote(ctx: Context) {
    // Create a channel to wait on config requests.
    let (send, recv) = channel();

    // Spawn a thread to receive config requests.
    let ctx_remote = ctx.clone();
    thread::Builder::new()
        .name("remote_service".to_string())
        .spawn(move || {
            run_remote_service(ctx_remote, send);
        })
        .expect("Failed to spawn remote service thread");

    loop {
        info!("Waiting for show configuration.");
        // Wait on a config from the remote service.
        let (config, run_flag) = recv.recv().expect("Remote service thread hung up.");

        info!("Starting a new show with configuration: {config:?}");
        // Start up a fresh show.
        match Show::new(config, ctx.clone(), run_flag) {
            Ok(mut show) => {
                info!("Show initialized, starting event loop.");
                // Run the show until the remote thread tells us to quit.
                show.run();
                info!("Show exited.");
            }

            // TODO: enable some kind of remote logging so we can collect these messages at the
            // controller.
            Err(e) => error!("Failed to initialize show: {e}"),
        }
    }
}

/// Run the remote discovery and configuration service, passing config states and cancellation
/// flags back to the main thread.
/// Panics if the service completes with an error.
pub fn run_remote_service(ctx: Context, sender: Sender<(ClientConfig, RunFlag)>) {
    // Run flag for currently-executing show, if there is one.
    let mut running_flag: Option<RunFlag> = None;

    run_service_req_rep(ctx, SERVICE_NAME, PORT, |request_buffer| {
        // Attempt to deserialize this request buffer as a client configuration.
        match deserialize_config(request_buffer) {
            Ok(config) => {
                // If there's currently a show running, pull the run flag out and stop it.
                let show_stop_message = if let Some(ref mut flag) = running_flag {
                    flag.stop();
                    "Stopped a running show."
                } else {
                    "No show was running."
                };

                // Create a new run control for the show we're about to start.
                let new_run_flag = RunFlag::default();
                running_flag = Some(new_run_flag.clone());

                // Send the config and flag back to the show thread.
                if let Err(e) = sender.send((config, new_run_flag)) {
                    format!("{show_stop_message}\nError trying to start new show: {e}.")
                } else {
                    // everything is OK
                    format!("{show_stop_message}\nStarting a new show.")
                }
            }
            Err(e) => format!("Could not parse request as a show configuration:\n{e}"),
        }
        .into_bytes()
    })
    .expect("Remote configuration service crashed")
}

fn deserialize_config(buffer: &[u8]) -> Result<ClientConfig, String> {
    from_read(buffer).map_err(|e| e.to_string())
}

// --- remote administration ---

/// Read a single line from stdin and return it as a string.
/// Panic if there's some IO-related error.
fn read_input() -> String {
    let mut val = String::new();
    stdin()
        .read_line(&mut val)
        .expect("Error trying to read user input");
    val.pop();
    val
}

/// Prompt a user for input and return the string they entered.
fn prompt_input(msg: &str) -> String {
    print!("{msg}");
    print!(": ");
    stdout().flush().expect("Error flushing stdout");
    read_input()
}

/// Repeatedly prompt the user for input until they provide an acceptable value.
/// Prints msg followed by a colon and a space.
fn prompt<P, T>(msg: &str, parser: P) -> T
where
    P: Fn(&str) -> Result<T, String>,
{
    loop {
        let input = prompt_input(msg);
        match parser(&input) {
            Ok(result) => {
                return result;
            }
            Err(e) => {
                println!("{e}");
            }
        }
    }
}

/// Extremely basic parsing of yes/no.
/// Accepts anything whose first letter is y or n, upper or lowercase.
fn parse_y_n(s: &str) -> Result<bool, String> {
    let lowered = s.to_lowercase();
    if lowered.starts_with('y') {
        Ok(true)
    } else if lowered.starts_with('n') {
        Ok(false)
    } else {
        Err(format!("Please enter y/n, not '{s}'."))
    }
}

/// Prompt for a yes/no answer.
fn prompt_y_n(msg: &str) -> bool {
    prompt(&format!("{msg}? Y/n"), parse_y_n)
}

/// Parse string as an unsigned integer.
fn parse_uint(s: &str) -> Result<u64, String> {
    s.parse()
        .map_err(|e| format!("Could not parse '{s}' as positive integer: {e}"))
}

/// Interactive series of user prompts, producing a configuration.
fn configure_one<H>(hostname: H) -> ClientConfig
where
    H: Into<String>,
{
    let video_channel = prompt("Select video channel", parse_uint);
    let resolution = prompt(
        "Specify display resolution (shorthands: wuxga (1920x1200), sx+ (1400x1050), heightp for 16:9, widthxheight)",
        parse_resolution,
    );
    let fullscreen = prompt_y_n("Fullscreen");
    let transformation = if prompt_y_n("Flip horizontal") {
        Some(Transform::Flip(TransformDirection::Horizontal))
    } else {
        None
    };

    let capture_mouse = prompt_y_n("Capture mouse");

    ClientConfig::new(
        video_channel,
        hostname.into(),
        resolution,
        fullscreen,
        capture_mouse,
        transformation,
        false,
    )
}

/// Slightly janky interactive command line utility for administering a fleet of tunnel clients.
pub fn administrate() {
    let host = hostname::get()
        .expect("Couldn't get hostname for this machine")
        .into_string()
        .unwrap();
    println!("Starting administrator...");
    let admin = Administrator::new(Context::new());

    // Wait a couple seconds for dns-sd to do its business.
    thread::sleep(Duration::from_secs(2));

    let usage = "l    List the available clients.
c    Configure a client.
q    Quit.";
    println!("Administrator started.");

    let parse_client_name = |name: &str| -> Result<String, String> {
        let clients = admin.clients();
        if clients.iter().any(|client| name == client) {
            Ok(name.to_string())
        } else {
            Err(format!(
                "'{}' is not a recognized client name; available clients: {}",
                name,
                clients.join("\n"),
            ))
        }
    };

    loop {
        println!("Commands:\n{usage}");
        match prompt_input("Enter a command").as_ref() {
            "l" => {
                println!("Available clients:\n{}\n", admin.clients().join("\n"));
            }
            "c" => {
                let client_name = prompt("Enter client name", parse_client_name);
                let config = configure_one(host.clone());
                match admin.run_with_config(&client_name, config) {
                    Ok(msg) => {
                        println!("{msg}");
                    }
                    Err(e) => {
                        println!("Could not configure due to an error: {e}");
                    }
                }
            }
            "q" => {
                break;
            }
            bad => {
                println!("Unknown command '{bad}'.");
            }
        }
    }
}
