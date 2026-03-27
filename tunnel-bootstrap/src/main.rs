//! Tunnel bootstrapper — receives binary pushes over ZMQ, writes them to disk,
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
use std::sync::Mutex;
use tunnels_lib::bootstrap::{PushBinaryRequest, PushBinaryResponse};
use zero_configure::req_rep::run_service_req_rep;
use zmq::Context;

const SERVICE_NAME: &str = "tunnelbootstrap";
const PORT: u16 = 15001;

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
    let run_args: Vec<&str> = request.run_args.iter().map(|s| s.as_str()).collect();
    let confirmation = manager
        .launch(&run_args, &request.stdin_payload)
        .context("Failed to launch")?;

    info!("Deployed successfully: {confirmation}");
    Ok(format!("Deployed successfully: {confirmation}"))
}

fn write_binary(path: &Path, payload: &[u8]) -> Result<()> {
    fs::write(path, payload)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    info!("Wrote managed binary ({} bytes)", payload.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Write a shell script that reads stdin, then prints "OK" and sleeps.
    fn write_fake_managed(dir: &Path) -> PathBuf {
        let path = dir.join("managed");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"#!/bin/sh
cat > /dev/null
echo "OK"
sleep 3600"#
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Write a shell script that always reports an error.
    fn write_failing_managed(dir: &Path) -> PathBuf {
        let path = dir.join("managed");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"#!/bin/sh
cat > /dev/null
echo "ERROR: something went wrong""#
        )
        .unwrap();
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
            run_args: vec!["monitor".to_string()],
            stdin_payload: vec![],
        };
        rmp_serde::to_vec(&request).unwrap()
    }

    fn make_bad_hash_request(payload: &[u8]) -> Vec<u8> {
        let request = PushBinaryRequest {
            sha256: [0u8; 32],
            payload: payload.to_vec(),
            run_args: vec!["monitor".to_string()],
            stdin_payload: vec![],
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
            run_args: vec!["monitor".to_string()],
            stdin_payload: vec![1, 2, 3],
        };

        let serialized = rmp_serde::to_vec(&request).unwrap();
        let deserialized: PushBinaryRequest = rmp_serde::from_slice(&serialized).unwrap();
        assert_eq!(deserialized.sha256, sha256);
        assert_eq!(deserialized.payload, payload);
        assert_eq!(deserialized.run_args, ["monitor"]);
        assert_eq!(deserialized.stdin_payload, [1, 2, 3]);

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
        assert!(msg.contains("Deployed"), "Unexpected message: {msg}");
        assert!(msg.contains("OK"), "Unexpected message: {msg}");

        assert!(manager.lock().unwrap().is_running());
    }

    #[test]
    fn test_child_error_response() {
        let dir = TempDir::new().unwrap();
        let managed_path = write_failing_managed(dir.path());
        let payload = fs::read(&managed_path).unwrap();
        let buffer = make_push_request(&payload);

        let manager = new_manager(&managed_path);
        let msg = handle_request(&buffer, &manager).unwrap();
        // The child prints "ERROR: ..." on stdout, which gets included in the response.
        assert!(
            msg.contains("ERROR: something went wrong"),
            "Unexpected message: {msg}"
        );
    }

    #[test]
    fn test_stdin_payload_piped_to_child() {
        let dir = TempDir::new().unwrap();
        let marker_path = dir.path().join("stdin_received");

        // Script that dumps stdin to a marker file, then prints OK.
        let managed_path = dir.path().join("managed");
        let mut f = fs::File::create(&managed_path).unwrap();
        writeln!(
            f,
            "#!/bin/sh\ncat > {}\necho OK\nsleep 3600",
            marker_path.display()
        )
        .unwrap();
        fs::set_permissions(&managed_path, fs::Permissions::from_mode(0o755)).unwrap();

        let payload = fs::read(&managed_path).unwrap();
        let stdin_data = b"hello from admin";

        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let sha256: [u8; 32] = hasher.finalize().into();
        let request = PushBinaryRequest {
            sha256,
            payload: payload.to_vec(),
            run_args: vec![],
            stdin_payload: stdin_data.to_vec(),
        };
        let buffer = rmp_serde::to_vec(&request).unwrap();

        let manager = new_manager(&managed_path);
        let msg = handle_request(&buffer, &manager).unwrap();
        assert!(msg.contains("OK"), "Unexpected message: {msg}");

        // Verify the stdin payload was received by the child.
        let received = fs::read_to_string(&marker_path).unwrap();
        assert_eq!(received, "hello from admin");
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
        assert!(msg.contains("Deployed"), "Unexpected message: {msg}");

        assert!(manager.lock().unwrap().is_running());
    }
}
