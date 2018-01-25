//! Enable remote control of a tunnel render slave over the network.
//! Advertise this slave for control over DNS-SD, handling requests on a 0mq socket.
//! Very basic control; every message received is a full configuration struct, and the receipt of
//! a message completely tears down an existing show and brings up a new one using the new
//! parameters.
//! Also provide the tools needed for simple remote administration.

use zero_configure::{run_service, Controller};
use zmq::Context;
use show::Show;
use config::ClientConfig;
use rmp_serde::decode::from_read;
use rmp_serde::encode::write;
use utils::RunFlag;
use std::thread;
use std::error::Error;

const SERVICE_NAME: &'static str = "tunnelclient";
const PORT: u16 = 15000;

// --- client remote control ---

/// Run this client as a remotely configurable service.
pub fn run_remote(ctx: &mut Context) {

    // Start out doing nothing.
    let mut running_show: Option<ShowManager> = None;

    run_service(SERVICE_NAME, PORT, |request_buffer| {
        // Attempt to deserialize this request buffer as a client configuration.
        match deserialize_config(request_buffer) {
            Ok(config) => {
                // Take ownership of the running show by swapping in None.
                let mut show_local = None;
                ::std::mem::swap(&mut show_local, &mut running_show);

                let show_stop_msg =
                    if let Some(show) = show_local {
                        match show.stop() {
                            Ok(()) => "Running show stopped cleanly.",
                            Err(()) => "Running show panicked.",
                        }
                    } else {
                        "No show was running."
                    };

                // start up a new show
                // FIXME this should return a Result if something went wrong starting the show.
                running_show = Some(ShowManager::new(config));

                // everything is OK
                format!("{}\nStarted a new show.", show_stop_msg)
            },
            Err(e) => format!("Could not parse request as a show configuration:\n{}", e),
        }.into_bytes()
    }).unwrap()
}

fn deserialize_config(buffer: &[u8]) -> Result<ClientConfig, String> {
    from_read(buffer).map_err(|e| e.to_string())
}

/// Handle to a show running on another thread.
struct ShowManager {
    show_thread: thread::JoinHandle<()>,
    run_flag: RunFlag,
}

impl ShowManager {
    /// Start up a new show using the provided configuration.
    /// Keep a handle to the thread the show is running in to allow us to gracefully wait for the
    /// show to terminate later.
    fn new(config: ClientConfig) -> Self {
        let run_flag = RunFlag::new();
        let run_flag_remote = run_flag.clone();

        println!("Starting with config:{:?}", config);

        let mut ctx = Context::new();

        let show_thread = thread::Builder::new()
            .name("running_show".to_string())
            .spawn(move || {
                let mut show = Show::new(config, &mut ctx, run_flag_remote);
                show.run();
            })
            .unwrap();

        ShowManager {
            show_thread,
            run_flag,
        }
    }

    /// Stop the running show.
    fn stop(mut self) -> Result<(), ()> {
        // Flip the run flag off.
        self.run_flag.stop();
        // Wait for the show thread to terminate.
        self.show_thread.join().map_err(|_| ())
    }
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
    pub fn run_with_config(&self, client: &str, config: ClientConfig) -> Result<String, Box<Error>> {
        // Serialize the config.
        let mut serialized = Vec::new();
        write(&mut serialized, &config)?;

        // Send the serialized command.
        let response = self.controller.send(client, &serialized)?;
        // Parse the string response.
        Ok(String::from_utf8(response)?)
    }

    // Command a particular client to run using a named configuration and other metadata.
}