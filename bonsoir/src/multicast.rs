//! Multicast UDP socket setup and network interface management.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

/// Multicast group address for bonsoir discovery.
/// Uses the mDNS multicast group (224.0.0.251) and port (5353) to match the
/// well-known mDNS configuration. Our packets are msgpack (not DNS), so
/// mDNSResponder ignores them. Using the same group/port as mDNS ensures
/// compatibility with the same socket options and OS-level multicast behavior
/// that mDNS relies on.
const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// Port for bonsoir multicast traffic.
const MULTICAST_PORT: u16 = 5353;

/// How often to re-check network interfaces for changes.
const INTERFACE_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// A UDP multicast socket that manages group membership across interfaces.
/// Owns the socket and the list of joined interfaces. Leaves the multicast
/// group on all interfaces when dropped.
pub(crate) struct MulticastSocket {
    socket: UdpSocket,
    interfaces: Vec<Ipv4Addr>,
    last_interface_check: std::time::Instant,
}

impl MulticastSocket {
    /// Create a new multicast socket, join the group on all interfaces.
    pub fn new() -> Result<Self> {
        let socket = create_socket()?;
        let interfaces = join_multicast(&socket);
        Ok(Self {
            socket,
            interfaces,
            last_interface_check: std::time::Instant::now(),
        })
    }

    /// Send data to the multicast group on every joined interface,
    /// setting IP_MULTICAST_IF before each send.
    pub fn send(&self, data: &[u8]) {
        let dest = SocketAddr::from((MULTICAST_ADDR, MULTICAST_PORT));
        for iface in &self.interfaces {
            if let Err(e) = socket2::SockRef::from(&self.socket).set_multicast_if_v4(iface) {
                log::debug!("[bonsoir] Failed to set IP_MULTICAST_IF to {iface}: {e}");
                continue;
            }
            if let Err(e) = self.socket.send_to(data, dest) {
                log::debug!("[bonsoir] Failed to send on {iface}: {e}");
            }
        }
    }

    /// Receive a packet. Non-blocking — returns WouldBlock if nothing available.
    pub fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.socket.recv_from(buf)
    }

    /// The list of interfaces we've joined multicast on. Used by the browser
    /// to detect same-machine services.
    pub fn interfaces(&self) -> &[Ipv4Addr] {
        &self.interfaces
    }

    /// Re-check network interfaces and rejoin if the set has changed.
    /// Call this periodically from the event loop.
    pub fn maybe_refresh_interfaces(&mut self) {
        if self.last_interface_check.elapsed() < INTERFACE_REFRESH_INTERVAL {
            return;
        }
        self.last_interface_check = std::time::Instant::now();

        let new_ifaces = all_multicast_interfaces();
        if new_ifaces == self.interfaces {
            return;
        }
        leave_multicast(&self.socket, &self.interfaces);
        self.interfaces = join_multicast(&self.socket);
        log::info!(
            "[bonsoir] Interfaces changed: {} -> {}",
            format_addrs(&new_ifaces),
            format_addrs(&self.interfaces),
        );
    }
}

impl Drop for MulticastSocket {
    fn drop(&mut self) {
        leave_multicast(&self.socket, &self.interfaces);
    }
}

/// Create a UDP socket configured for multicast send/receive.
fn create_socket() -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to set SO_REUSEADDR")?;
    #[cfg(unix)]
    if let Err(e) = socket.set_reuse_port(true) {
        log::debug!("[bonsoir] SO_REUSEPORT not supported, continuing: {e}");
    }
    socket
        .set_nonblocking(true)
        .context("failed to set O_NONBLOCK")?;
    socket
        .set_multicast_loop_v4(true)
        .context("failed to set IP_MULTICAST_LOOP")?;
    socket
        .set_multicast_ttl_v4(255)
        .context("failed to set IP_MULTICAST_TTL")?;
    socket
        .bind(&SockAddr::from(SocketAddr::from((
            Ipv4Addr::UNSPECIFIED,
            MULTICAST_PORT,
        ))))
        .context("failed to bind multicast socket")?;
    Ok(socket.into())
}

/// Join the multicast group on all available interfaces.
fn join_multicast(socket: &UdpSocket) -> Vec<Ipv4Addr> {
    let mut joined = Vec::new();
    for iface in all_multicast_interfaces() {
        match socket.join_multicast_v4(&MULTICAST_ADDR, &iface) {
            Ok(()) => {
                log::debug!("[bonsoir] Joined multicast on {iface}");
                joined.push(iface);
            }
            Err(e) => {
                log::debug!("[bonsoir] Failed to join multicast on {iface}: {e}");
            }
        }
    }
    if joined.is_empty() {
        log::warn!("[bonsoir] Could not join multicast on any interface");
    }
    joined
}

/// Leave the multicast group on the given interfaces.
fn leave_multicast(socket: &UdpSocket, interfaces: &[Ipv4Addr]) {
    for iface in interfaces {
        let _ = socket.leave_multicast_v4(&MULTICAST_ADDR, iface);
    }
}

/// All IPv4 interfaces we should join multicast on: every non-loopback
/// interface plus loopback itself (for same-machine discovery and CI).
fn all_multicast_interfaces() -> Vec<Ipv4Addr> {
    let mut ifaces: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| {
            if iface.is_loopback() {
                return None;
            }
            match iface.ip() {
                IpAddr::V4(addr) => Some(addr),
                IpAddr::V6(_) => None,
            }
        })
        .collect();
    if !ifaces.contains(&Ipv4Addr::LOCALHOST) {
        ifaces.push(Ipv4Addr::LOCALHOST);
    }
    ifaces
}

fn format_addrs(addrs: &[Ipv4Addr]) -> String {
    let strs: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
    format!("[{}]", strs.join(", "))
}
