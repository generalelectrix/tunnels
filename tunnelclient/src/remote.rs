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

const SERVICE_NAME: &'static str = "tunnelclient";
const PORT: u16 = 15000;

fn deserialize_config(buffer: &[u8]) -> Result<ClientConfig, String> {
    from_read(buffer).map_err(|e| e.to_string())
}

/// Run this client as a remotely configurable service.
fn run_remote(ctx: &mut Context) {

    let mut show: Option<Show> = None;

    run_service(SERVICE_NAME, PORT, |request_buffer| {
        // Attempt to deserialize this request buffer as a client configuration.
        match deserialize_config(request_buffer) {
            Ok(config) => {
                // tear down an existing show if one is running
                // start up a new show
                // everything is OK
                "Ok.".to_string()
            },
            Err(e) => e,
        }.into_bytes()
    });
}