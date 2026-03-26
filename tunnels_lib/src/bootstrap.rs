//! Protocol types for the tunnel-bootstrap binary push system.

use serde::{Deserialize, Serialize};

/// Payload for a binary push.
#[derive(Serialize, Deserialize)]
pub struct PushBinaryRequest {
    pub sha256: [u8; 32],
    pub payload: Vec<u8>,
    /// Args to pass for the health check (e.g. `["self-test"]`).
    pub health_check_args: Vec<String>,
    /// Args to pass when launching the binary (e.g. `["remote"]`).
    pub run_args: Vec<String>,
}

/// Response from the bootstrapper: Ok(message) on success, Err(reason) on failure.
pub type PushBinaryResponse = Result<String, String>;
