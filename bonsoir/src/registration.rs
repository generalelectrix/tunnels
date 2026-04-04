//! Service registration: periodic heartbeats on the multicast group.

use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::multicast;
use crate::wire::Packet;

/// How often to send heartbeat packets.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

/// How often to re-check network interfaces.
const INTERFACE_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// A registered service that heartbeats on the multicast group.
/// Dropping this sends goodbye packets for clean deregistration.
pub struct Registration {
    shutdown_tx: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl Registration {
    /// Register a service and start heartbeating.
    ///
    /// - `service_type`: short name like `"tunnelbootstrap"` (max 15 bytes)
    /// - `instance_name`: human-readable name like `"Bore A"` (max 63 bytes)
    /// - `port`: the TCP port the service listens on
    pub fn new(service_type: &str, instance_name: &str, port: u16) -> Result<Self> {
        let socket = multicast::sender_socket().context("registration socket")?;
        // Also bind a receiver socket so we can hear Query packets.
        let recv_socket = multicast::receiver_socket().context("registration recv socket")?;
        recv_socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .context("set read timeout")?;
        let joined = multicast::join_multicast(&recv_socket);

        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        let service_type = service_type.to_string();
        let instance_name = instance_name.to_string();
        let dest = multicast::multicast_dest();

        log::info!(
            "[bonsoir] Service '{service_type}' ('{instance_name}') registered on port {port}",
        );

        let join_handle = thread::spawn(move || {
            let mut interfaces = joined;
            let mut last_heartbeat = Instant::now() - HEARTBEAT_INTERVAL; // send immediately
            let mut last_interface_check = Instant::now();
            let mut buf = [0u8; 256];

            loop {
                // Check for shutdown.
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }

                // Send heartbeat if due.
                if last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
                    send_heartbeat(&socket, &service_type, &instance_name, port, &dest);
                    last_heartbeat = Instant::now();
                }

                // Refresh interfaces if due.
                if last_interface_check.elapsed() >= INTERFACE_REFRESH_INTERVAL {
                    interfaces = multicast::refresh_interfaces(&recv_socket, &interfaces);
                    last_interface_check = Instant::now();
                }

                // Listen for Query packets (non-blocking with timeout).
                match recv_socket.recv_from(&mut buf) {
                    Ok((len, _src)) => {
                        if let Ok(pkt) = Packet::decode(&buf[..len]) {
                            if pkt.message_type == crate::wire::MessageType::Query
                                && pkt.service_type == service_type
                            {
                                log::debug!(
                                    "[bonsoir] Received query for '{}', responding",
                                    service_type
                                );
                                send_heartbeat(
                                    &socket,
                                    &service_type,
                                    &instance_name,
                                    port,
                                    &dest,
                                );
                                last_heartbeat = Instant::now();
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(e) => {
                        log::warn!("[bonsoir] Registration recv error: {e}");
                    }
                }
            }

            // Send goodbye packets (3x for redundancy).
            let goodbye = Packet::goodbye(&service_type, &instance_name, port);
            if let Ok(bytes) = goodbye.encode() {
                for _ in 0..3 {
                    let _ = socket.send_to(&bytes, dest);
                    thread::sleep(Duration::from_millis(50));
                }
            }
            log::info!(
                "[bonsoir] Service '{}' ('{}') deregistered",
                service_type,
                instance_name
            );

            multicast::leave_multicast(&recv_socket, &interfaces);
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

fn send_heartbeat(
    socket: &std::net::UdpSocket,
    service_type: &str,
    instance_name: &str,
    port: u16,
    dest: &std::net::SocketAddr,
) {
    let pkt = Packet::heartbeat(service_type, instance_name, port);
    match pkt.encode() {
        Ok(bytes) => {
            if let Err(e) = socket.send_to(&bytes, dest) {
                log::warn!("[bonsoir] Failed to send heartbeat: {e}");
            }
        }
        Err(e) => {
            log::warn!("[bonsoir] Failed to encode heartbeat: {e}");
        }
    }
}
