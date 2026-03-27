//! Advertise a service over DNS-SD. Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using TCP request-response.

use anyhow::bail;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::time::Duration;

use anyhow::Result;

use crate::bare::{register_service, Browser};

/// Advertise a service over DNS-SD, using TCP request-response as the transport.
/// Pass each message received on the socket to the action callback. Send the byte
/// buffer returned by the action callback back to the requester.
pub fn run_service_req_rep<F>(name: &str, port: u16, action: F) -> Result<()>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
    let _registration = register_service(name, port)?;
    minusmq::req_rep::serve(listener, action)
}

/// Maintain a collection of service instances we can remotely interact with.
/// Communication is performed via TCP request-response pairs.
pub struct Controller {
    browser: Browser<SocketAddr>,
    timeout: Option<Duration>,
}

impl Controller {
    /// Start up a new service controller at the given service name.
    /// Asynchronously browse for new services, and remove them if they deregister.
    pub fn new(name: String) -> Self {
        Self::with_recv_timeout(name, None)
    }

    /// Start up a new service controller with an optional timeout.
    pub fn with_recv_timeout(name: String, timeout: Option<Duration>) -> Self {
        Self {
            browser: Browser::new(name, |service| {
                resolve_addr(&service.host_target, service.port)
            }),
            timeout,
        }
    }

    /// List the services currently available.
    pub fn list(&self) -> Vec<String> {
        self.browser.list()
    }

    /// Send a message to one of the services on this controller, returning the response.
    pub fn send(&self, name: &str, msg: &[u8]) -> Result<Vec<u8>> {
        let timeout = self.timeout;
        self.browser
            .use_service(name, |addr| {
                if let Some(t) = timeout {
                    minusmq::req_rep::send_with_timeout(*addr, msg, t)
                } else {
                    minusmq::req_rep::send(*addr, msg)
                }
            })
            .unwrap_or_else(|| bail!(format!("No service named '{}' available.", name)))
    }
}

/// Resolve a hostname:port to a SocketAddr at discovery time.
/// Prefers IPv4 addresses since our listeners bind to 0.0.0.0.
fn resolve_addr(host: &str, port: u16) -> Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
    // Prefer IPv4 since our listeners bind to 0.0.0.0.
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve {host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration};

    /// Return a byte vector containing DEADBEEF.
    fn deadbeef() -> Vec<u8> {
        vec![0xD, 0xE, 0xA, 0xD, 0xB, 0xE, 0xE, 0xF]
    }

    /// Return a byte vector containing 0123.
    fn testbytes() -> Vec<u8> {
        vec![0, 1, 2, 3]
    }

    fn sleep(dt: u64) {
        thread::sleep(Duration::from_millis(dt))
    }

    /// Test that we can advertise a single service and successfully connect to it.
    #[test]
    fn test_pair() {
        let name = "test";
        let port = 10000;

        let controller = Controller::new(name.to_string());

        // Wait a moment, and assert that we can't see any services.
        sleep(500);

        assert!(controller.list().is_empty());

        // Start up the service; return DEADBEEF as a response.
        thread::spawn(move || {
            run_service_req_rep(name, port, |buffer| {
                assert_eq!(testbytes(), buffer);
                deadbeef()
            })
            .unwrap();
        });

        // Give the service a moment to register via DNS-SD and start listening.
        sleep(3000);

        let names = controller.list();
        assert_eq!(1, names.len());

        // Send with a retry — the TCP listener might not be fully ready
        // even after DNS-SD discovery succeeds.
        let mut response = None;
        for _ in 0..5 {
            match controller.send(&names[0], &testbytes()) {
                Ok(r) => {
                    response = Some(r);
                    break;
                }
                Err(_) => sleep(500),
            }
        }
        assert_eq!(response.unwrap(), deadbeef());
    }
}
