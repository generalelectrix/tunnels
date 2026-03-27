//! Child process management for the bootstrapper.
//!
//! Manages the lifecycle of the "managed" binary: launching, stopping,
//! and monitoring with exponential backoff restart.

use anyhow::{Context, Result};
use log::{error, info, warn};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BACKOFF_CAP: Duration = Duration::from_secs(30);
const STABILITY_THRESHOLD: Duration = Duration::from_secs(60);
const STDOUT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ChildManager {
    child: Option<Child>,
    args: Vec<String>,
    stdin_payload: Vec<u8>,
    /// Path to the managed binary.
    binary_path: PathBuf,
    /// When the current child was launched.
    launched_at: Option<Instant>,
    /// Current backoff duration for restart.
    backoff: Duration,
}

impl ChildManager {
    pub fn new(binary_path: &Path) -> Self {
        Self {
            child: None,
            args: Vec::new(),
            stdin_payload: Vec::new(),
            binary_path: binary_path.to_path_buf(),
            launched_at: None,
            backoff: Duration::from_secs(1),
        }
    }

    /// The path to the managed binary.
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    /// Kill the currently running child process, if any.
    pub fn stop(&mut self) {
        let Some(ref mut child) = self.child else {
            return;
        };

        info!("Killing child process (pid {})", child.id());
        let _ = child.kill();
        let _ = child.wait();

        self.child = None;
        self.launched_at = None;
    }

    /// Launch the managed binary with the given arguments, piping `stdin_payload`
    /// to its stdin and reading the first line of stdout as a status response.
    pub fn launch(&mut self, args: &[&str], stdin_payload: &[u8]) -> Result<String> {
        self.args = args.iter().map(|s| s.to_string()).collect();
        self.stdin_payload = stdin_payload.to_vec();
        self.spawn()
    }

    fn spawn(&mut self) -> Result<String> {
        info!(
            "Launching {} {}",
            self.binary_path.display(),
            self.args.join(" ")
        );
        let mut child = Command::new(&self.binary_path)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("Failed to spawn managed binary")?;

        info!("Child started (pid {})", child.id());

        // Write stdin payload and close the pipe.
        {
            let mut stdin = child.stdin.take().unwrap();
            stdin
                .write_all(&self.stdin_payload)
                .context("Failed to write stdin payload to child")?;
        }

        // Read the first line of stdout with a timeout.
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = tx.send(line);
        });

        let line = rx
            .recv_timeout(STDOUT_TIMEOUT)
            .context("Timed out waiting for child status on stdout")?;

        let line = line.trim().to_string();
        info!("Child status: {line}");

        self.child = Some(child);
        self.launched_at = Some(Instant::now());

        Ok(line)
    }

    /// Check if the child is still running. If it has exited, attempt a restart
    /// with exponential backoff. Returns true if the child needed a restart.
    #[expect(unused)]
    pub fn monitor(&mut self) -> bool {
        let Some(ref mut child) = self.child else {
            return false;
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                warn!("Child exited unexpectedly with {status}");
                self.child = None;

                // Reset backoff if the process was stable for long enough.
                if let Some(launched) = self.launched_at {
                    if launched.elapsed() >= STABILITY_THRESHOLD {
                        self.backoff = Duration::from_secs(1);
                    }
                }

                info!("Restarting in {:?}", self.backoff);
                std::thread::sleep(self.backoff);

                // Increase backoff for next time.
                self.backoff = (self.backoff * 2).min(BACKOFF_CAP);

                if let Err(e) = self.spawn() {
                    error!("Failed to restart child: {e}");
                }
                true
            }
            Ok(None) => false, // Still running.
            Err(e) => {
                error!("Error checking child status: {e}");
                false
            }
        }
    }

    /// Returns true if a child process is currently running.
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for ChildManager {
    fn drop(&mut self) {
        self.stop();
        if let Err(e) = std::fs::remove_file(&self.binary_path) {
            warn!("Could not remove managed binary: {e}");
        }
    }
}
