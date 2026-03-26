//! Host-side controller for discovering and pushing binaries to bootstrappers.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tunnels_lib::bootstrap::{PushBinaryRequest, PushBinaryResponse};
use zero_configure::req_rep::Controller;
use zmq::Context;

const SERVICE_NAME: &str = "tunnelbootstrap";

/// Discovers bootstrapper instances on the LAN and pushes binaries to them.
pub struct BootstrapController {
    controller: Controller,
}

impl BootstrapController {
    pub fn new(ctx: Context) -> Self {
        Self {
            controller: Controller::new(ctx, SERVICE_NAME.to_string()),
        }
    }

    /// List discovered bootstrapper instances.
    pub fn list(&self) -> Vec<String> {
        self.controller.list()
    }

    /// Push a binary to a named bootstrapper instance.
    /// `health_check_args`: args for the health check (empty to skip).
    /// `run_args`: args to launch the binary with.
    pub fn push_binary(
        &self,
        name: &str,
        binary_path: &Path,
        health_check_args: &[&str],
        run_args: &[&str],
    ) -> Result<String> {
        let payload = fs::read(binary_path)?;

        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let sha256: [u8; 32] = hasher.finalize().into();

        let request = PushBinaryRequest {
            sha256,
            payload,
            health_check_args: health_check_args.iter().map(|s| s.to_string()).collect(),
            run_args: run_args.iter().map(|s| s.to_string()).collect(),
        };

        let serialized = rmp_serde::to_vec(&request)?;
        let response_bytes = self.controller.send(name, &serialized)?;
        let response: PushBinaryResponse = rmp_serde::from_slice(&response_bytes)?;
        response.map_err(|e| anyhow::anyhow!(e))
    }
}
