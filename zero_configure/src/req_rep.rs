//! Advertise a service via bonsoir. Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using TCP request-response.

use anyhow::bail;
pub use minusmq::req_rep::Config;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};

use anyhow::Result;

use crate::bare::{Browser, create_and_register};

/// Advertise a service via bonsoir, using TCP request-response as the transport.
/// Pass each message received on the socket to the action callback. Send the byte
/// buffer returned by the action callback back to the requester.
///
/// A request declaring more than the configured message length is refused
/// before anything is sized to it.
pub fn run_service_req_rep<F>(name: &str, port: u16, config: Config, action: F) -> Result<()>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
    // Keep _registration alive on the stack; dropping it would end the heartbeats.
    let (_registration, _instance_name) = create_and_register(name, port)?;
    minusmq::req_rep::serve(listener, config, action)
}

/// Maintain a collection of service instances we can remotely interact with.
/// Communication is performed via TCP request-response pairs.
pub struct Controller {
    browser: Browser<SocketAddr>,
    /// How an exchange with one of these services is carried.
    config: Config,
}

impl Controller {
    /// Start up a new service controller at the given service name, whose
    /// exchanges `config` describes.
    /// Asynchronously browse for new services, and remove them when they expire.
    pub fn new(name: String, config: Config) -> Self {
        Self {
            browser: Browser::new(name, |service| {
                resolve_addr(&service.hostname, service.port)
            }),
            config,
        }
    }

    /// List the services currently available.
    pub fn list(&self) -> Vec<String> {
        self.browser.list()
    }

    /// Send a message to one of the services on this controller, returning the response.
    pub fn send(&self, name: &str, msg: &[u8]) -> Result<Vec<u8>> {
        let config = self.config;
        self.browser
            .use_service(name, |addr| minusmq::req_rep::send(*addr, msg, config))
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

    /// A limit no message in a test reaches, for the tests that are about
    /// something other than the limit.
    const TEST_MAX_MESSAGE_LEN: usize = 64 * 1024;

    /// An exchange of the size a test sends, waiting as long as it takes.
    fn test_config() -> Config {
        Config {
            max_message_len: TEST_MAX_MESSAGE_LEN,
            ..Default::default()
        }
    }

    /// Test that we can advertise a single service and successfully connect to it.
    #[test]
    fn test_pair() {
        let _ = env_logger::builder().is_test(true).try_init();
        let name = "reqreptest";
        let port = 19992;

        let controller = Controller::new(name.to_string(), test_config());

        // Wait a moment, and assert that we can't see any services.
        sleep(500);

        assert!(controller.list().is_empty());

        // Start up the service; return DEADBEEF as a response.
        thread::spawn(move || {
            run_service_req_rep(name, port, test_config(), |buffer| {
                assert_eq!(testbytes(), buffer);
                deadbeef()
            })
            .unwrap();
        });

        // Give the service a moment to register and start heartbeating.
        // bonsoir heartbeats every 2s, so we need at least that plus startup time.
        sleep(5000);

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
