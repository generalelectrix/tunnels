//! Service registration: periodic heartbeats on the multicast group.

use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::multicast::MulticastSocket;
use crate::wire::{Packet, RECV_BUF_SIZE};
use crate::Timing;

/// Number of goodbye packets to send on shutdown. Multiple copies provide
/// redundancy since UDP delivery is not guaranteed.
const GOODBYE_REPEAT_COUNT: usize = 3;

/// Delay between repeated goodbye packets.
const GOODBYE_REPEAT_DELAY: Duration = Duration::from_millis(50);

/// A registered service that heartbeats on the multicast group.
/// Dropping this sends goodbye packets for clean deregistration.
pub struct Registration {
    shutdown_tx: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl Registration {
    /// Register a service and start heartbeating with default timing.
    pub fn new(service_type: &str, instance_name: &str, port: u16) -> Result<Self> {
        Self::with_timing(service_type, instance_name, port, Timing::default())
    }

    /// Register a service with custom timing.
    pub fn with_timing(
        service_type: &str,
        instance_name: &str,
        port: u16,
        timing: Timing,
    ) -> Result<Self> {
        let socket = MulticastSocket::new()?;

        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        log::info!(
            "[bonsoir] Service '{service_type}' ('{instance_name}') registered on port {port}",
        );

        let service_type = service_type.to_string();
        let instance_name = instance_name.to_string();

        let join_handle = thread::spawn(move || {
            registration_loop(
                socket,
                &service_type,
                &instance_name,
                port,
                timing,
                shutdown_rx,
            );
        });

        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(join_handle),
        })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

fn registration_loop(
    mut socket: MulticastSocket,
    service_type: &str,
    instance_name: &str,
    port: u16,
    timing: Timing,
    shutdown_rx: std::sync::mpsc::Receiver<()>,
) {
    let mut last_heartbeat = Instant::now() - timing.heartbeat_interval; // send immediately
    let mut buf = [0u8; RECV_BUF_SIZE];
    let poll_interval = timing.heartbeat_interval / 2;

    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        if last_heartbeat.elapsed() >= timing.heartbeat_interval {
            send_heartbeat(&socket, service_type, instance_name, port);
            last_heartbeat = Instant::now();
        }

        socket.maybe_refresh_interfaces();

        // Non-blocking recv — sleep briefly if nothing available.
        match socket.recv_from(&mut buf) {
            Ok((len, _src)) => {
                if let Ok(pkt) = Packet::decode(&buf[..len]) {
                    if pkt.message_type == crate::wire::MessageType::Query
                        && pkt.service_type == service_type
                    {
                        log::debug!(
                            "[bonsoir] Received query for '{}', responding",
                            service_type
                        );
                        send_heartbeat(&socket, service_type, instance_name, port);
                        last_heartbeat = Instant::now();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(poll_interval);
            }
            Err(e) => {
                log::warn!("[bonsoir] Registration recv error: {e}");
                thread::sleep(poll_interval);
            }
        }
    }

    // Send goodbye packets for redundancy.
    let goodbye = Packet::goodbye(service_type, instance_name, port);
    if let Ok(bytes) = goodbye.encode() {
        for _ in 0..GOODBYE_REPEAT_COUNT {
            socket.send(&bytes);
            thread::sleep(GOODBYE_REPEAT_DELAY);
        }
    }
    log::info!("[bonsoir] Service '{service_type}' ('{instance_name}') deregistered");
}

fn send_heartbeat(socket: &MulticastSocket, service_type: &str, instance_name: &str, port: u16) {
    let pkt = Packet::heartbeat(service_type, instance_name, port);
    match pkt.encode() {
        Ok(bytes) => socket.send(&bytes),
        Err(e) => log::warn!("[bonsoir] Failed to encode heartbeat: {e}"),
    }
}
