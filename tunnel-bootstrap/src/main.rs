//! Tunnel bootstrapper — receives binary pushes over ZMQ, health-checks them,
//! and launches them as managed child processes.

mod child;

use anyhow::{Context as _, Result};
use child::ChildManager;
use log::{error, info};
use sha2::{Digest, Sha256};
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use tunnels_lib::bootstrap::{PushBinaryRequest, PushBinaryResponse};
use zero_configure::req_rep::run_service_req_rep;
use zmq::Context;

const SERVICE_NAME: &str = "tunnelbootstrap";
const PORT: u16 = 15001;
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

fn main() {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())
        .expect("Could not configure logger");

    info!("tunnel-bootstrap starting on port {PORT}");

    let ctx = Context::new();
    let managed_path = Path::new("./managed");
    let child_manager = Mutex::new(ChildManager::new(managed_path));

    run_service_req_rep(ctx, SERVICE_NAME, PORT, |request_buffer| {
        let response = handle_request(request_buffer, &child_manager);
        rmp_serde::to_vec(&response).unwrap_or_else(|e| {
            error!("Failed to serialize response: {e}");
            Vec::new()
        })
    })
    .expect("Bootstrap service crashed");
}

fn handle_request(buffer: &[u8], child_manager: &Mutex<ChildManager>) -> PushBinaryResponse {
    let request: PushBinaryRequest = match rmp_serde::from_slice(buffer) {
        Ok(req) => req,
        Err(e) => return Err(format!("Failed to deserialize request: {e}")),
    };
    handle_push(request, child_manager).map_err(|e| e.to_string())
}

fn handle_push(request: PushBinaryRequest, child_manager: &Mutex<ChildManager>) -> Result<String> {
    info!("Received push: payload={} bytes", request.payload.len());

    // Verify SHA-256.
    let mut hasher = Sha256::new();
    hasher.update(&request.payload);
    let computed: [u8; 32] = hasher.finalize().into();
    anyhow::ensure!(computed == request.sha256, "SHA-256 mismatch");

    let mut manager = child_manager.lock().unwrap();

    // Stop existing child if running.
    if manager.is_running() {
        info!("Stopping existing child before update");
        manager.stop();
    }

    let binary_path = manager.binary_path().to_path_buf();

    write_binary(&binary_path, &request.payload).context("Failed to write binary")?;
    if !request.health_check_args.is_empty() {
        run_health_check(&binary_path, &request.health_check_args)
            .context("Health check failed")?;
    }
    let run_args: Vec<&str> = request.run_args.iter().map(|s| s.as_str()).collect();
    manager.launch(&run_args).context("Failed to launch")?;

    info!("Deployed successfully");
    Ok("Deployed successfully".to_string())
}

fn write_binary(path: &Path, payload: &[u8]) -> Result<()> {
    fs::write(path, payload)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    info!("Wrote managed binary ({} bytes)", payload.len());
    Ok(())
}

fn run_health_check(binary_path: &Path, args: &[String]) -> Result<()> {
    info!(
        "Running health check: {} {}",
        binary_path.display(),
        args.join(" ")
    );
    let mut child = Command::new(binary_path).args(args).spawn()?;

    let deadline = std::time::Instant::now() + HEALTH_CHECK_TIMEOUT;
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => {
                info!("Health check passed");
                return Ok(());
            }
            Some(status) => {
                anyhow::bail!("self-test exited with {status}");
            }
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    anyhow::bail!("self-test timed out after {HEALTH_CHECK_TIMEOUT:?}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_fake_managed(dir: &Path) -> PathBuf {
        let path = dir.join("managed");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"#!/bin/sh
case "$1" in
    self-test) exit 0 ;;
    remote) sleep 3600 ;;
    *) exit 1 ;;
esac"#
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn write_failing_managed(dir: &Path) -> PathBuf {
        let path = dir.join("managed");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh\nexit 1").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn new_manager(managed_path: &Path) -> Mutex<ChildManager> {
        Mutex::new(ChildManager::new(managed_path))
    }

    fn make_push_request(payload: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let sha256: [u8; 32] = hasher.finalize().into();
        let request = PushBinaryRequest {
            sha256,
            payload: payload.to_vec(),
            health_check_args: vec!["self-test".to_string()],
            run_args: vec!["remote".to_string()],
        };
        rmp_serde::to_vec(&request).unwrap()
    }

    fn make_bad_hash_request(payload: &[u8]) -> Vec<u8> {
        let request = PushBinaryRequest {
            sha256: [0u8; 32],
            payload: payload.to_vec(),
            health_check_args: vec!["self-test".to_string()],
            run_args: vec!["remote".to_string()],
        };
        rmp_serde::to_vec(&request).unwrap()
    }

    #[test]
    fn test_protocol_round_trip() {
        let payload = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let sha256: [u8; 32] = hasher.finalize().into();

        let request = PushBinaryRequest {
            sha256,
            payload: payload.to_vec(),
            health_check_args: vec!["self-test".to_string()],
            run_args: vec!["remote".to_string()],
        };

        let serialized = rmp_serde::to_vec(&request).unwrap();
        let deserialized: PushBinaryRequest = rmp_serde::from_slice(&serialized).unwrap();
        assert_eq!(deserialized.sha256, sha256);
        assert_eq!(deserialized.payload, payload);
        assert_eq!(deserialized.health_check_args, ["self-test"]);
        assert_eq!(deserialized.run_args, ["remote"]);

        // Round-trip Ok and Err responses.
        let ok: PushBinaryResponse = Ok("deployed".to_string());
        let ser = rmp_serde::to_vec(&ok).unwrap();
        let de: PushBinaryResponse = rmp_serde::from_slice(&ser).unwrap();
        assert_eq!(de.unwrap(), "deployed");

        let err: PushBinaryResponse = Err("bad hash".to_string());
        let ser = rmp_serde::to_vec(&err).unwrap();
        let de: PushBinaryResponse = rmp_serde::from_slice(&ser).unwrap();
        assert_eq!(de.unwrap_err(), "bad hash");
    }

    #[test]
    fn test_sha256_mismatch_rejected() {
        let dir = TempDir::new().unwrap();
        let managed_path = write_fake_managed(dir.path());
        let manager = new_manager(&managed_path);

        let buffer = make_bad_hash_request(b"some binary content");
        let err = handle_request(&buffer, &manager).unwrap_err();
        assert!(err.contains("SHA-256 mismatch"));
    }

    #[test]
    fn test_invalid_request_rejected() {
        let dir = TempDir::new().unwrap();
        let managed_path = dir.path().join("managed");
        let manager = new_manager(&managed_path);

        let err = handle_request(b"not valid msgpack", &manager).unwrap_err();
        assert!(err.contains("Failed to deserialize"));
    }

    #[test]
    fn test_successful_push() {
        let dir = TempDir::new().unwrap();
        let managed_path = write_fake_managed(dir.path());
        let payload = fs::read(&managed_path).unwrap();
        let buffer = make_push_request(&payload);

        let manager = new_manager(&managed_path);
        let msg = handle_request(&buffer, &manager).unwrap();
        assert!(msg.contains("Deployed"));

        assert!(manager.lock().unwrap().is_running());
    }

    #[test]
    fn test_health_check_failure_rejects_push() {
        let dir = TempDir::new().unwrap();
        let managed_path = write_failing_managed(dir.path());
        let payload = fs::read(&managed_path).unwrap();
        let buffer = make_push_request(&payload);

        let manager = new_manager(&managed_path);
        let err = handle_request(&buffer, &manager).unwrap_err();
        assert!(
            err.contains("Health check failed"),
            "Unexpected error: {err}"
        );

        assert!(!manager.lock().unwrap().is_running());
    }

    #[test]
    fn test_child_stop_and_relaunch() {
        let dir = TempDir::new().unwrap();
        let managed_path = write_fake_managed(dir.path());
        let payload = fs::read(&managed_path).unwrap();

        let manager = new_manager(&managed_path);

        let buffer = make_push_request(&payload);
        handle_request(&buffer, &manager).unwrap();

        let buffer = make_push_request(&payload);
        let msg = handle_request(&buffer, &manager).unwrap();
        assert!(msg.contains("Deployed"));

        assert!(manager.lock().unwrap().is_running());
    }
}
