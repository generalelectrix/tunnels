//! Protocol types for the tunnel-bootstrap binary push system.

use serde::{Deserialize, Serialize};

/// Payload for a binary push.
#[derive(Serialize, Deserialize)]
pub struct PushBinaryRequest {
    pub sha256: [u8; 32],
    pub payload: Vec<u8>,
    /// Args to pass when launching the binary (e.g. `["monitor"]`).
    pub run_args: Vec<String>,
    /// Data to pipe into the child's stdin after launch (e.g. serialized config).
    pub stdin_payload: Vec<u8>,
}

/// Response from the bootstrapper: Ok(message) on success, Err(reason) on failure.
pub type PushBinaryResponse = Result<String, String>;
