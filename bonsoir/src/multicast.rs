//! Multicast UDP socket setup and network interface management.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

/// Multicast group address for bonsoir discovery.
/// Uses the mDNS multicast group (224.0.0.251) so that macOS exempts our
/// traffic from the application firewall. Our packets are msgpack (not DNS),
/// so mDNSResponder ignores them.
pub const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// Port for bonsoir multicast traffic. Uses the mDNS port for the same
/// firewall exemption reason.
pub const MULTICAST_PORT: u16 = 5353;

/// Create a UDP socket for both sending and receiving multicast packets.
/// Binds to INADDR_ANY on the multicast port. Uses a single socket for
/// send and receive, matching the pattern used by mDNSResponder and the
/// mdns-sd crate for reliable same-machine multicast on macOS.
pub fn multicast_socket() -> Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to set SO_REUSEADDR")?;
    #[cfg(unix)]
    socket
        .set_reuse_port(true)
        .context("failed to set SO_REUSEPORT")?;
    // Enable multicast loopback for same-machine delivery.
    socket
        .set_multicast_loop_v4(true)
        .context("failed to set IP_MULTICAST_LOOP")?;
    // Set multicast TTL to 255 per RFC 6762.
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

/// The multicast destination address for sending packets.
pub fn multicast_dest() -> SocketAddr {
    SocketAddr::from((MULTICAST_ADDR, MULTICAST_PORT))
}

/// Join the multicast group on all available IPv4 interfaces, including loopback.
/// Loopback is always included to ensure same-machine discovery works in
/// environments with no external network (e.g. CI containers).
/// Returns the list of interface addresses we joined on.
pub fn join_multicast(socket: &std::net::UdpSocket) -> Vec<Ipv4Addr> {
    let mut joined = Vec::new();
    // Always include loopback for same-machine discovery.
    let mut ifaces = ipv4_interfaces();
    if !ifaces.contains(&Ipv4Addr::LOCALHOST) {
        ifaces.push(Ipv4Addr::LOCALHOST);
    }
    for iface in ifaces {
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
