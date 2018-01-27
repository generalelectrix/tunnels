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
use std::sync::mpsc::{channel, Sender};

const SERVICE_NAME: &'static str = "tunnelclient";
const PORT: u16 = 15000;

// --- client remote control ---

/// Run this client as a remotely configurable service.
/// The show starts up in the main thread to ensure we don't end up with issues trying to pass
/// OpenGL resources between threads.
/// Spawn a second thread to run the remote service, passing configurations to run back across a
/// channel.
/// Panics if the remote service thread fails to spawn.
pub fn run_remote(ctx: &mut Context) {
    // Create a channel to wait on config requests.
    let (send, recv) = channel();

    // Spawn a thread to receive config requests.
    thread::Builder::new().name("remote_service".to_string()).spawn(|| {
        let mut ctx = Context::new();
        run_remote_service(&mut ctx, send);
    }).expect("Failed to spawn remote service thread");

    loop {
        println!("Waiting for show configuration.");
        // Wait on a config from the remote service.
        let (config, run_flag) = recv.recv().expect("Remote service thread hung up.");

        println!("Starting a new show with configuration: {:?}", config);
        // Start up a fresh show.
        match Show::new(config, ctx, run_flag) {
            Ok(mut show) => {
                println!("Show initialized, starting event loop.");
                // Run the show until the remote thread tells us to quit.
                show.run();
                println!("Show exited.");
            },

            // TODO: enable some kind of remote logging so we can collect these messages at the
            // controller.
            Err(e) => println!("Failed to initialize show: {}", e),
        }


    }
}

/// Run the remote discovery and configuration service, passing config states and cancellation
/// flags back to the main thread.
/// Panics if the service completes with an error.
pub fn run_remote_service(ctx: &mut Context, sender: Sender<(ClientConfig, RunFlag)>) {

    // Run flag for currently-executing show, if there is one.
    let mut running_flag: Option<RunFlag> = None;

    run_service(SERVICE_NAME, PORT, |request_buffer| {

        // Attempt to deserialize this request buffer as a client configuration.
        match deserialize_config(request_buffer) {
            Ok(config) => {

                // If there's currently a show running, pull the run flag out and stop it.
                let show_stop_message =
                    if let Some(ref mut flag) = running_flag {
                        flag.stop();
                        "Stopped a running show."
                    } else {
                        "No show was running."
                    };

                // Create a new run control for the show we're about to start.
                let new_run_flag = RunFlag::new();
                running_flag = Some(new_run_flag.clone());

                // Send the config and flag back to the show thread.
                sender.send((config, new_run_flag));

                // everything is OK
                format!("{}\nStarting a new show.", show_stop_message)
            },
            Err(e) => format!("Could not parse request as a show configuration:\n{}", e),
        }.into_bytes()
    }).expect("Remote configuration service crashed")
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
    pub fn run_with_config(&self, client: &str, config: ClientConfig) -> Result<String, Box<Error>> {
        // Serialize the config.
        let mut serialized = Vec::new();
        write(&mut serialized, &config)?;

        // Send the serialized command.
        let response = self.controller.send(client, &serialized)?;
        // Parse the string response.
        Ok(String::from_utf8(response)?)
    }

    // /// Command a particular client to run using a named configuration and other metadata.
    //pub fn run(&self, client: &str, video_channel: u64, config)
}