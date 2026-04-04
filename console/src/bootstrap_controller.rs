//! Host-side controller for discovering and pushing binaries to bootstrappers.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::time::Duration;
use tunnels_lib::bootstrap::{PushBinaryRequest, PushBinaryResponse};
use zero_configure::req_rep::Controller;

const SERVICE_NAME: &str = "tunnelbootstrap";

/// Discovers bootstrapper instances on the LAN and pushes binaries to them.
pub struct BootstrapController {
    controller: Controller,
}

impl BootstrapController {
    /// Create a bootstrap controller using the provided timeout.
    ///
    /// If None, push actions will block until they complete or explicitly fail.
    pub fn new(timeout: Option<Duration>) -> Self {
        Self {
            controller: Controller::with_recv_timeout(SERVICE_NAME.to_string(), timeout),
        }
    }

    /// List discovered bootstrapper instances.
    pub fn list(&self) -> Vec<String> {
        self.controller.list()
    }

    /// Force-restart discovery, clearing stale services and re-browsing from scratch.
    pub fn refresh(&self) {
        self.controller.refresh();
    }

    /// Push a binary to a named bootstrapper instance.
    /// `run_args`: args to launch the binary with.
    /// `stdin_payload`: data to pipe into the child's stdin after launch.
    pub fn push_binary(
        &self,
        name: &str,
        binary_path: &Path,
        run_args: &[&str],
        stdin_payload: &[u8],
    ) -> Result<String> {
        let payload = fs::read(binary_path)?;

        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let sha256: [u8; 32] = hasher.finalize().into();

        let request = PushBinaryRequest {
            sha256,
            payload,
            run_args: run_args.iter().map(|s| s.to_string()).collect(),
            stdin_payload: stdin_payload.to_vec(),
        };

        let serialized = rmp_serde::to_vec(&request)?;
        let response_bytes = self.controller.send(name, &serialized)?;
        let response: PushBinaryResponse = rmp_serde::from_slice(&response_bytes)?;
        response.map_err(|e| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_registers() {
        // Use an arbitrary port; we only need to verify the service name is valid.
        let stop = zero_configure::bare::register_service(SERVICE_NAME, 0)
            .expect("should register");
        stop();
    }
}
