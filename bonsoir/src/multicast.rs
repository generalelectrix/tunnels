//! Multicast UDP socket setup and network interface management.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

/// Multicast group address for bonsoir discovery.
/// 239.255.66.83 — site-local scope (RFC 2365). 66/83 = ASCII 'B'/'S'.
pub const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 66, 83);

/// Port for bonsoir multicast traffic.
pub const MULTICAST_PORT: u16 = 5765;

/// Create a UDP socket suitable for sending multicast packets.
/// The socket is bound to an ephemeral port on all interfaces.
pub fn sender_socket() -> Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to set SO_REUSEADDR")?;
    #[cfg(unix)]
    socket
        .set_reuse_port(true)
        .context("failed to set SO_REUSEPORT")?;
    socket
        .bind(&SockAddr::from(SocketAddr::from((
            Ipv4Addr::UNSPECIFIED,
            0,
        ))))
        .context("failed to bind sender socket")?;
    Ok(socket.into())
}

/// Create a UDP socket suitable for receiving multicast packets.
/// Binds to the multicast port with address reuse enabled.
pub fn receiver_socket() -> Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to set SO_REUSEADDR")?;
    #[cfg(unix)]
    socket
        .set_reuse_port(true)
        .context("failed to set SO_REUSEPORT")?;
    socket
        .bind(&SockAddr::from(SocketAddr::from((
            Ipv4Addr::UNSPECIFIED,
            MULTICAST_PORT,
        ))))
        .context("failed to bind receiver socket")?;
    Ok(socket.into())
}

/// The multicast destination address for sending packets.
pub fn multicast_dest() -> SocketAddr {
    SocketAddr::from((MULTICAST_ADDR, MULTICAST_PORT))
}

/// Join the multicast group on all available IPv4 interfaces.
/// Returns the list of interface addresses we joined on.
pub fn join_multicast(socket: &std::net::UdpSocket) -> Vec<Ipv4Addr> {
    let mut joined = Vec::new();
    for iface in ipv4_interfaces() {
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
pub fn leave_multicast(socket: &std::net::UdpSocket, interfaces: &[Ipv4Addr]) {
    for iface in interfaces {
        let _ = socket.leave_multicast_v4(&MULTICAST_ADDR, iface);
    }
}

/// Enumerate all non-loopback IPv4 interface addresses.
pub fn ipv4_interfaces() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
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
        .collect()
}

/// Rejoin multicast if the set of interfaces has changed.
/// Returns the updated interface list.
pub fn refresh_interfaces(
    socket: &std::net::UdpSocket,
    current: &[Ipv4Addr],
) -> Vec<Ipv4Addr> {
    let new_ifaces = ipv4_interfaces();
    if new_ifaces == current {
        return current.to_vec();
    }
    // Leave old, join new.
    leave_multicast(socket, current);
    let joined = join_multicast(socket);
    log::info!(
        "[bonsoir] Interfaces changed: {} -> {}",
        format_addrs(current),
        format_addrs(&joined),
    );
    joined
}

fn format_addrs(addrs: &[Ipv4Addr]) -> String {
    let strs: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
    format!("[{}]", strs.join(", "))
}
