//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using 0mq REQ/REP sockets.

use anyhow::bail;
use mdns_sd::{ServiceDaemon, ServiceInfo};

use zmq::{Context, Socket};

use anyhow::Result;
use std::collections::HashMap;

use crate::bare::{mdns_hostname, service_type_fq, Browser};

/// Advertise a service over DNS-SD, using a 0mq REQ/REP socket as the subsequent transport.
/// Pass each message received on the socket to the action callback.  Send the byte buffer returned
/// by the action callback back to the requester.
pub fn run_service_req_rep<F>(ctx: Context, name: &str, port: u16, mut action: F) -> Result<()>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    // Open the 0mq socket we'll use to service requests.
    let socket = ctx.socket(zmq::REP)?;
    let addr = format!("tcp://*:{port}");
    socket.bind(&addr)?;

    // Start advertising this service over DNS-SD.
    let service_type = service_type_fq(name);
    let daemon = ServiceDaemon::new()?;

    let hostname = mdns_hostname();

    let service_info = ServiceInfo::new(
        &service_type,
        name,
        &hostname,
        "",
        port,
        None::<HashMap<String, String>>,
    )?
    .enable_addr_auto();
    daemon.register(service_info)?;

    loop {
        if let Ok(msg) = socket.recv_bytes(0) {
            if let Err(e) = socket.send(action(&msg), 0) {
                println!("Failed to send response: {e}");
            }
        }
    }
}

/// Maintain a collection of service instances we can remotely interact with.
/// Communication is performed via 0mq REQ/REP pairs.
pub struct Controller(Browser<Socket>);

impl Controller {
    /// Start up a new service controller at the given service name.
    /// Asynchronously browse for new services, and remove them if they deregister.
    /// For the moment, panic if anything goes wrong during initialization.
    /// This is acceptable as this action will run once during startup and there's nothing to do
    /// except bail completely if this process fails.
    pub fn new(ctx: Context, name: String) -> Self {
        Self::with_recv_timeout(ctx, name, None)
    }

    /// Start up a new service controller with an optional receive timeout.
    /// If `recv_timeout` is `Some(ms)`, REQ sockets will time out after `ms` milliseconds
    /// and be configured with REQ_RELAXED + REQ_CORRELATE to remain usable after a timeout.
    /// If `None`, sockets block forever on receive (the default ZMQ behavior).
    pub fn with_recv_timeout(ctx: Context, name: String, recv_timeout: Option<i32>) -> Self {
        Self(Browser::new(name, move |service| {
            req_socket(&service.hostname, service.port, &ctx, recv_timeout)
        }))
    }

    /// List the services currently available.
    pub fn list(&self) -> Vec<String> {
        self.0.list()
    }

    /// Send a message to one of the services on this controller, returning the response.
    pub fn send(&self, name: &str, msg: &[u8]) -> Result<Vec<u8>> {
        self.0
            .use_service(name, |socket| {
                socket.send(msg, 0)?;
                let response = socket.recv_bytes(0)?;
                Ok(response)
            })
            .unwrap_or_else(|| bail!(format!("No service named '{}' available.", name)))
    }
}

/// Try to connect a REQ socket at this host and port.
/// If `recv_timeout` is `Some(ms)`, configure the socket with a receive timeout and
/// REQ_RELAXED + REQ_CORRELATE so it remains usable after a timeout.
fn req_socket(host: &str, port: u16, ctx: &Context, recv_timeout: Option<i32>) -> Result<Socket> {
    let addr = format!("tcp://{host}:{port}");

    // Connect a REQ socket.
    let socket = ctx.socket(zmq::REQ)?;
    if let Some(ms) = recv_timeout {
        socket.set_rcvtimeo(ms)?;
        // REQ_RELAXED: allow sending a new request before receiving a reply (prevents EFSM
        // error after a receive timeout).
        socket.set_req_relaxed(true)?;
        // REQ_CORRELATE: match replies to requests by ID, preventing stale replies from a
        // timed-out request from being delivered to a subsequent request.
        socket.set_req_correlate(true)?;
    }
    socket.connect(&addr)?;
    Ok(socket)
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
        let name = "reqreptest";
        let port = 19992;

        let controller = Controller::new(Context::new(), name.to_string());

        // Wait a moment, and assert that we can't see any services.
        sleep(500);

        assert!(controller.list().is_empty());

        // Start up the service; return DEADBEEF as a response.
        thread::spawn(move || {
            run_service_req_rep(Context::new(), name, port, |buffer| {
                assert_eq!(testbytes(), buffer);
                deadbeef()
            })
            .unwrap();
        });

        // Give the service a moment to get situated.
        sleep(2000);

        let names = controller.list();
        assert_eq!(1, names.len());

        // Test sending a message.
        let response = controller.send(&names[0], &testbytes()).unwrap();

        assert_eq!(deadbeef(), response);
    }
}
