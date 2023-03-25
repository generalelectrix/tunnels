//! Enable remote control of a tunnel render slave over the network.
//! Advertise this slave for control over DNS-SD, handling requests on a 0mq socket.
//! Very basic control; every message received is a full configuration struct, and the receipt of
//! a message completely tears down an existing show and brings up a new one using the new
//! parameters.
//! Also provide the tools needed for simple remote administration.

use crate::config::{ClientConfig, Resolution};
use crate::draw::{Transform, TransformDirection};
use crate::show::Show;
use hostname;
use lazy_static::lazy_static;
use log::{error, info};
use regex::Regex;
use rmp_serde::decode::from_read;
use rmp_serde::encode::write;
use std::error::Error;
use std::io::{stdin, stdout, Write};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;
use tunnels_lib::RunFlag;
use zero_configure::{run_service_req_rep, Controller};
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

        info!("Starting a new show with configuration: {:?}", config);
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
            Err(e) => error!("Failed to initialize show: {}", e),
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
                let new_run_flag = RunFlag::new();
                running_flag = Some(new_run_flag.clone());

                // Send the config and flag back to the show thread.
                if let Err(e) = sender.send((config, new_run_flag)) {
                    format!(
                        "{}\nError trying to start new show: {}.",
                        show_stop_message, e
                    )
                } else {
                    // everything is OK
                    format!("{}\nStarting a new show.", show_stop_message)
                }
            }
            Err(e) => format!("Could not parse request as a show configuration:\n{}", e),
        }
        .into_bytes()
    })
    .expect("Remote configuration service crashed")
}

fn deserialize_config(buffer: &[u8]) -> Result<ClientConfig, String> {
    from_read(buffer).map_err(|e| e.to_string())
}

// --- remote administration ---

/// Provide an API for administering a flock of tunnel clients.
pub struct Administrator {
    /// zero_configure service controller.
    controller: Controller,
}

impl Administrator {
    pub fn new() -> Self {
        Administrator {
            controller: Controller::new(SERVICE_NAME),
        }
    }

    /// Return the list of clients that are currently available.
    pub fn clients(&self) -> Vec<String> {
        self.controller.list()
    }

    /// Command a particular client to run using the provided configuration.
    /// If the client is available, returns the string response from sending the config.
    /// Returns Err if the specified client doesn't exist.
    pub fn run_with_config(
        &self,
        client: &str,
        config: ClientConfig,
    ) -> Result<String, Box<dyn Error>> {
        // Serialize the config.
        let mut serialized = Vec::new();
        write(&mut serialized, &config)?;

        // Send the serialized command.
        let response = self.controller.send(client, &serialized)?;
        // Parse the string response.
        Ok(String::from_utf8(response)?)
    }
}

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
    print!("{}", msg);
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
                println!("{}", e);
            }
        }
    }
}

/// Parse an expression as a resolution.
/// Understands some shorthand like 1080p but also parses widthxheight.
fn parse_resolution(res_str: &str) -> Result<Resolution, String> {
    lazy_static! {
        static ref SHORTHAND_RE: Regex = Regex::new(r"^(\d+)p$").unwrap();
        static ref WIDTH_HEIGHT_RE: Regex = Regex::new(r"^(\d+)x(\d+)$").unwrap();
    }
    // Try matching against shorthand.
    if let Some(caps) = SHORTHAND_RE.captures(res_str) {
        let height: u32 = caps[1]
            .parse()
            .expect("Regex should only have matched integers");
        let width = height * 16 / 9;
        return Ok((width, height));
    }
    // Try matching generic expression.
    if let Some(caps) = WIDTH_HEIGHT_RE.captures(res_str) {
        let width: u32 = caps[1]
            .parse()
            .expect("Regex should only have matched integers.");
        let height: u32 = caps[2]
            .parse()
            .expect("Regex should only have matched integers.");
        return Ok((width, height));
    }
    let res_str = res_str.to_lowercase();
    // Normalize input and check against whitelist.
    match res_str.as_ref() {
        "sxga+" | "sx+" => Ok((1400, 1050)),
        _ => Err(format!(
            "Couldn't parse {} as resolution expression.",
            res_str
        )),
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
        Err(format!("Please enter y/n, not '{}'.", s))
    }
}

/// Prompt for a yes/no answer.
fn prompt_y_n(msg: &str) -> bool {
    prompt(&format!("{}? Y/n", msg), parse_y_n)
}

/// Parse string as an unsigned integer.
fn parse_uint(s: &str) -> Result<u64, String> {
    s.parse()
        .map_err(|e| format!("Could not parse '{}' as positive integer: {}", s, e))
}

/// Parse string as float.
fn parse_f64(s: &str) -> Result<f64, String> {
    s.parse()
        .map_err(|e| format!("Could not parse '{}' as float: {}", s, e))
}

/// Interactive series of user prompts, producing a configuration.
fn configure_one<H>(hostname: H) -> ClientConfig
where
    H: Into<String>,
{
    let video_channel = prompt("Select video channel", parse_uint);
    let resolution = prompt(
        "Specify display resolution (widthxheight or heightp for 16:9)",
        parse_resolution,
    );
    let fullscreen = prompt_y_n("Fullscreen");
    let transformation = if prompt_y_n("Flip horizontal") {
        Some(Transform::Flip(TransformDirection::Horizontal))
    } else {
        None
    };

    // Some defaults we might configure in advanced mode.
    let mut anti_alias = true;
    let mut timesync_interval = Duration::from_secs(60);
    let mut render_delay = 0.015;
    let mut alpha_blend = true;
    let mut capture_mouse = true;

    if prompt_y_n("Configure advanced settings") {
        capture_mouse = prompt_y_n("Capture mouse");
        anti_alias = prompt_y_n("Use anti-aliasing");
        alpha_blend = prompt_y_n("Use alpha channel blending");
        let timesync_interval_secs = prompt(
            "Host/client time resynchronization interval in seconds (default 60)",
            parse_uint,
        );
        timesync_interval = Duration::from_secs(timesync_interval_secs);
        render_delay = prompt("Client render delay in seconds (default 0.015)", parse_f64);
    }

    ClientConfig::new(
        video_channel,
        hostname.into(),
        resolution,
        timesync_interval,
        Duration::from_secs_f64(render_delay),
        anti_alias,
        fullscreen,
        alpha_blend,
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
    let admin = Administrator::new();

    // Wait a couple seconds for dns-sd to do its business.
    thread::sleep(Duration::from_secs(2));

    let usage = "list    List the available clients.
conf    Configure a client.
quit    Quit.";
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
        println!("Commands:\n{}", usage);
        match prompt_input("Enter a command").as_ref() {
            "list" | "l" => {
                println!("Available clients:\n{}\n", admin.clients().join("\n"));
            }
            "conf" | "c" => {
                let client_name = prompt("Enter client name", &parse_client_name);
                let config = configure_one(host.clone());
                match admin.run_with_config(&client_name, config) {
                    Ok(msg) => {
                        println!("{}", msg);
                    }
                    Err(e) => {
                        println!("Could not configure due to an error: {}", e);
                    }
                }
            }
            "quit" | "q" => {
                break;
            }
            bad => {
                println!("Unknown command '{}'.", bad);
            }
        }
    }
}
