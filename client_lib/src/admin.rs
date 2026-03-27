//! API for administering a flock of tunnel clients over the network.

use crate::config::{ClientConfig, Resolution};
use anyhow::Result;
use lazy_static::lazy_static;
use regex::Regex;
use rmp_serde::encode::write;
use zero_configure::req_rep::Controller;
use zmq::Context;

const SERVICE_NAME: &str = "tunnelclient";

/// Provide an API for administering a flock of tunnel clients.
pub struct Administrator {
    /// zero_configure service controller.
    controller: Controller,
}

impl Administrator {
    pub fn new(ctx: Context) -> Self {
        Administrator {
            controller: Controller::new(ctx, SERVICE_NAME.to_string()),
        }
    }

    /// Create an administrator with a receive timeout on the underlying ZMQ sockets.
    /// If a client fails to respond within `timeout_ms` milliseconds, the send will
    /// return an error instead of blocking forever.
    pub fn with_recv_timeout(ctx: Context, timeout_ms: i32) -> Self {
        Administrator {
            controller: Controller::with_recv_timeout(
                ctx,
                SERVICE_NAME.to_string(),
                Some(timeout_ms),
            ),
        }
    }

    /// Return the list of clients that are currently available.
    pub fn clients(&self) -> Vec<String> {
        self.controller.list()
    }

    /// Command a particular client to run using the provided configuration.
    /// If the client is available, returns the string response from sending the config.
    /// Returns Err if the specified client doesn't exist.
    pub fn run_with_config(&self, client: &str, config: ClientConfig) -> Result<String> {
        // Serialize the config.
        let mut serialized = Vec::new();
        write(&mut serialized, &config)?;

        // Send the serialized command.
        let response = self.controller.send(client, &serialized)?;
        // Parse the string response.
        Ok(String::from_utf8(response)?)
    }
}

/// Parse an expression as a resolution.
/// Understands some shorthand like 1080p but also parses widthxheight.
pub fn parse_resolution(res_str: &str) -> Result<Resolution, String> {
    lazy_static! {
        static ref WIDESCREEN_RE: Regex = Regex::new(r"^(\d+)p$").unwrap();
        static ref WIDTH_HEIGHT_RE: Regex = Regex::new(r"^(\d+)x(\d+)$").unwrap();
    }
    // Try matching against shorthand.
    if let Some(caps) = WIDESCREEN_RE.captures(res_str) {
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
        "wuxga" => Ok((1920, 1200)),
        "sxga+" | "sx+" => Ok((1400, 1050)),
        _ => Err(format!(
            "Couldn't parse {res_str} as resolution expression."
        )),
    }
}
