//! Service browser: listen for heartbeats, track liveness, expire stale entries.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::multicast;
use crate::wire::{MessageType, Packet, ServiceInstance, PROTOCOL_VERSION};

/// How long before a service is considered expired (3 missed heartbeats).
const EXPIRY_TIMEOUT: Duration = Duration::from_secs(6);

/// How often to check for expired services.
const EXPIRY_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// How often to re-check network interfaces.
const INTERFACE_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Events emitted by the browser.
#[derive(Debug, Clone)]
pub enum BrowseEvent {
    /// A service appeared or updated its info.
    ServiceUp(ServiceInstance),
    /// A service disappeared (expired or said goodbye). Contains instance name.
    ServiceDown(String),
}

/// Tracks a discovered service's liveness.
struct TrackedService {
    last_seen: Instant,
}

/// A browser that listens for heartbeats and tracks service liveness.
/// Dropping this stops the listen thread.
pub struct Browser {
    shutdown_tx: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl Browser {
    /// Start browsing for services of the given type.
    /// Returns a `Browser` handle and a `Receiver` that emits `BrowseEvent`s.
    /// Immediately sends a Query to solicit fast responses from existing services.
    pub fn new(service_type: &str) -> Result<(Self, Receiver<BrowseEvent>)> {
        let socket = multicast::receiver_socket().context("browser socket")?;
        socket
            .set_read_timeout(Some(EXPIRY_CHECK_INTERVAL))
            .context("set read timeout")?;
        let joined = multicast::join_multicast(&socket);

        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        // Send initial query.
        let query = Packet::query(service_type);
        if let Ok(bytes) = query.encode() {
            let dest = multicast::multicast_dest();
            if let Err(e) = socket.send_to(&bytes, dest) {
                log::warn!("[bonsoir] Failed to send initial query: {e}");
            }
        }

        log::info!("[bonsoir] Browsing for '{service_type}'");

        let service_type = service_type.to_string();

        let join_handle = thread::spawn(move || {
            browse_loop(socket, &service_type, joined, shutdown_rx, event_tx);
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
    socket: std::net::UdpSocket,
    service_type: &str,
    initial_interfaces: Vec<std::net::Ipv4Addr>,
    shutdown_rx: Receiver<()>,
    event_tx: Sender<BrowseEvent>,
) {
    let mut services: HashMap<String, TrackedService> = HashMap::new();
    let mut interfaces = initial_interfaces;
    let mut last_expiry_check = Instant::now();
    let mut last_interface_check = Instant::now();
    let mut buf = [0u8; 512];

    loop {
        // Check for shutdown.
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        // Receive packets (blocks up to EXPIRY_CHECK_INTERVAL).
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                match Packet::decode(&buf[..len]) {
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
                            continue; // Not our service type.
                        }
                        handle_packet(&pkt, src.ip(), &mut services, &event_tx);
                    }
                    Err(e) => {
                        log::debug!("[bonsoir] Failed to decode packet from {src}: {e}");
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                log::warn!("[bonsoir] Browser recv error: {e}");
            }
        }

        // Expire stale services.
        if last_expiry_check.elapsed() >= EXPIRY_CHECK_INTERVAL {
            expire_stale(&mut services, &event_tx);
            last_expiry_check = Instant::now();
        }

        // Refresh interfaces.
        if last_interface_check.elapsed() >= INTERFACE_REFRESH_INTERVAL {
            interfaces = multicast::refresh_interfaces(&socket, &interfaces);
            last_interface_check = Instant::now();
        }
    }

    multicast::leave_multicast(&socket, &interfaces);
}

fn handle_packet(
    pkt: &Packet,
    sender_addr: std::net::IpAddr,
    services: &mut HashMap<String, TrackedService>,
    event_tx: &Sender<BrowseEvent>,
) {
    match pkt.message_type {
        MessageType::Heartbeat => {
            let instance = ServiceInstance {
                service_type: pkt.service_type.clone(),
                instance_name: pkt.instance_name.clone(),
                address: sender_addr,
                port: pkt.port,
            };
            let is_new = !services.contains_key(&pkt.instance_name);
            services.insert(
                pkt.instance_name.clone(),
                TrackedService {
                    last_seen: Instant::now(),
                },
            );
            if is_new {
                log::info!(
                    "[bonsoir] Discovered '{}' at {}:{}",
                    pkt.instance_name,
                    sender_addr,
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
        MessageType::Query => {
            // Browsers ignore queries.
        }
    }
}

fn expire_stale(
    services: &mut HashMap<String, TrackedService>,
    event_tx: &Sender<BrowseEvent>,
) {
    let expired: Vec<String> = services
        .iter()
        .filter(|(_, tracked)| tracked.last_seen.elapsed() > EXPIRY_TIMEOUT)
        .map(|(name, _)| name.clone())
        .collect();

    for name in expired {
        services.remove(&name);
        log::info!("[bonsoir] '{}' expired (no heartbeat for {:?})", name, EXPIRY_TIMEOUT);
        let _ = event_tx.send(BrowseEvent::ServiceDown(name));
    }
}
