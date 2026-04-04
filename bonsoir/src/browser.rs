//! Service browser: listen for heartbeats, track liveness, expire stale entries.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::Result;

use crate::multicast::MulticastSocket;
use crate::wire::{MessageType, Packet, ServiceInstance, PROTOCOL_VERSION, RECV_BUF_SIZE};
use crate::Timing;

/// Events emitted by the browser.
#[derive(Debug, Clone)]
pub enum BrowseEvent {
    /// A service appeared or updated its address/port.
    ServiceUp(ServiceInstance),
    /// A service disappeared (expired or said goodbye). Contains instance name.
    ServiceDown(String),
}

/// Tracks a discovered service's liveness and identity.
struct TrackedService {
    address: IpAddr,
    port: u16,
    last_seen: Instant,
}

/// A browser that listens for heartbeats and tracks service liveness.
/// Dropping this stops the listen thread.
pub struct Browser {
    shutdown_tx: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl Browser {
    /// Start browsing with default timing.
    pub fn new(service_type: &str) -> Result<(Self, Receiver<BrowseEvent>)> {
        Self::with_timing(service_type, Timing::default())
    }

    /// Start browsing with custom timing.
    pub fn with_timing(
        service_type: &str,
        timing: Timing,
    ) -> Result<(Self, Receiver<BrowseEvent>)> {
        let socket = MulticastSocket::new()?;

        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        // Send initial query to solicit fast responses from existing services.
        let query = Packet::query(service_type);
        if let Ok(bytes) = query.encode() {
            socket.send(&bytes);
        }

        log::info!("[bonsoir] Browsing for '{service_type}'");

        let service_type = service_type.to_string();

        let join_handle = thread::spawn(move || {
            browse_loop(socket, &service_type, shutdown_rx, event_tx, timing);
        });

        Ok((
            Self {
                shutdown_tx: Some(shutdown_tx),
                join_handle: Some(join_handle),
            },
            event_rx,
        ))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

fn browse_loop(
    mut socket: MulticastSocket,
    service_type: &str,
    shutdown_rx: Receiver<()>,
    event_tx: Sender<BrowseEvent>,
    timing: Timing,
) {
    let mut services: HashMap<String, TrackedService> = HashMap::new();
    let mut last_expiry_check = Instant::now();
    let mut buf = [0u8; RECV_BUF_SIZE];
    let expiry_check_interval = timing.expiry_timeout / 3;

    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        match socket.recv_from(&mut buf) {
            Ok((len, src)) => match Packet::decode(&buf[..len]) {
                Ok(pkt) => {
                    if pkt.version != PROTOCOL_VERSION {
                        log::warn!(
                            "[bonsoir] Dropping packet from {}: version {}, expected {}",
                            src.ip(),
                            pkt.version,
                            PROTOCOL_VERSION
                        );
                        continue;
                    }
                    if pkt.service_type != service_type {
                        continue;
                    }
                    handle_packet(
                        &pkt,
                        src.ip(),
                        socket.interfaces(),
                        &mut services,
                        &event_tx,
                    );
                }
                Err(e) => {
                    log::debug!("[bonsoir] Failed to decode packet from {src}: {e}");
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(expiry_check_interval);
            }
            Err(e) => {
                log::warn!("[bonsoir] Browser recv error: {e}");
                thread::sleep(expiry_check_interval);
            }
        }

        if last_expiry_check.elapsed() >= expiry_check_interval {
            expire_stale(&mut services, &event_tx, &timing);
            last_expiry_check = Instant::now();
        }

        socket.maybe_refresh_interfaces();
    }
}

fn handle_packet(
    pkt: &Packet,
    sender_addr: IpAddr,
    local_interfaces: &[Ipv4Addr],
    services: &mut HashMap<String, TrackedService>,
    event_tx: &Sender<BrowseEvent>,
) {
    match pkt.message_type {
        MessageType::Heartbeat => {
            // Use loopback for services on our own machine.
            let address = match sender_addr {
                IpAddr::V4(v4) if local_interfaces.contains(&v4) => IpAddr::V4(Ipv4Addr::LOCALHOST),
                other => other,
            };

            // Emit ServiceUp if this is a new service or its address/port changed.
            let should_emit = match services.get(&pkt.instance_name) {
                None => true,
                Some(tracked) => tracked.address != address || tracked.port != pkt.port,
            };

            services.insert(
                pkt.instance_name.clone(),
                TrackedService {
                    address,
                    port: pkt.port,
                    last_seen: Instant::now(),
                },
            );

            if should_emit {
                let instance = ServiceInstance {
                    service_type: pkt.service_type.clone(),
                    instance_name: pkt.instance_name.clone(),
                    address,
                    port: pkt.port,
                };
                log::info!(
                    "[bonsoir] Discovered '{}' at {}:{}",
                    pkt.instance_name,
                    address,
                    pkt.port
                );
                let _ = event_tx.send(BrowseEvent::ServiceUp(instance));
            }
        }
        MessageType::Goodbye => {
            if services.remove(&pkt.instance_name).is_some() {
                log::info!("[bonsoir] '{}' said goodbye", pkt.instance_name);
                let _ = event_tx.send(BrowseEvent::ServiceDown(pkt.instance_name.clone()));
            }
        }
        MessageType::Query => {}
    }
}

fn expire_stale(
    services: &mut HashMap<String, TrackedService>,
    event_tx: &Sender<BrowseEvent>,
    timing: &Timing,
) {
    let expired: Vec<String> = services
        .iter()
        .filter(|(_, tracked)| tracked.last_seen.elapsed() > timing.expiry_timeout)
        .map(|(name, _)| name.clone())
        .collect();

    for name in expired {
        services.remove(&name);
        log::info!(
            "[bonsoir] '{}' expired (no heartbeat for {:?})",
            name,
            timing.expiry_timeout
        );
        let _ = event_tx.send(BrowseEvent::ServiceDown(name));
    }
}
