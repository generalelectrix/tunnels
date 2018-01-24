/// Enable remote control of a tunnel render slave over the network.
/// Advertises this slave for control over DNS-SD, handling requests on a 0mq socket.
/// Very basic control; every message received is a full configuration struct, and the receipt of
/// a message completely tears down an existing show and brings up a new one using the new
/// parameters.

use zero_configure::run_service;
use zmq::Context;
use show::Show;
use config::ClientConfig;
use rmp_serde::decode::from_read;
use std::error::Error;
use utils::RunFlag;
use std::thread;

const SERVICE_NAME: &'static str = "tunnelclient";
const PORT: u16 = 15000;

fn deserialize_config(buffer: &[u8]) -> Result<ClientConfig, String> {
    from_read(buffer).map_err(|e| e.to_string())
}

/// Run this client as a remotely configurable service.
fn run_remote(ctx: &mut Context) {

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
                    if let Some(mut show) = show_local {
                        match show.stop() {
                            Ok(()) => "Running show stopped cleanly.",
                            Err(()) => "Running show panicked.",
                        }
                    } else {
                        "No show was running."
                    };

                // start up a new show
                running_show = Some(ShowManager::new(config, ctx));

                // everything is OK
                "Ok.".to_string()
            },
            Err(e) => format!("Could not parse request as a show configuration:\n{}", e),
        }.into_bytes()
    });
}

struct ShowManager {
    show_thread: thread::JoinHandle<()>,
    run_flag: RunFlag,
}

impl ShowManager {
    /// Start up a new show using the provided configuration.
    /// Keep a handle to the thread the show is running in to allow us to gracefully wait for the
    /// show to terminate later.
    fn new(config: ClientConfig, ctx: &mut Context) -> Self {
        let run_flag = RunFlag::new();
        let mut show = Show::new(config, ctx, run_flag.clone());
        let show_thread = thread::spawn(move || {
            show.run();
        });

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