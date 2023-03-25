//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using 0mq REQ/REP sockets.

use async_dnssd::{register_extended, RegisterData, RegisterFlags};
use simple_error::bail;
use tokio_core::reactor::Core;

use zmq::{Context, Socket};

use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::bare::{browse_forever, reg_type};

/// Advertise a service over DNS-SD, using a 0mq REQ/REP socket as the subsequent transport.
/// Pass each message received on the socket to the action callback.  Send the byte buffer returned
/// by the action callback back to the requester.
pub fn run_service_req_rep<F>(
    ctx: Context,
    name: &str,
    port: u16,
    mut action: F,
) -> Result<(), Box<dyn Error>>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    // Open the 0mq socket we'll use to service requests.
    let socket = ctx.socket(zmq::REP)?;
    let addr = format!("tcp://*:{}", port);
    socket.bind(&addr)?;

    // Create a tokio core just to run this one future.
    let core = Core::new()?;

    // Start advertising this service over DNS-SD.
    let mut register_data = RegisterData::default();
    register_data.flags = RegisterFlags::SHARED;
    let _registration = register_extended(&reg_type(name), port, register_data, &core.handle())?;

    loop {
        if let Ok(msg) = socket.recv_bytes(0) {
            if let Err(e) = socket.send(&action(&msg), 0) {
                println!("Failed to send response: {}", e);
            }
        }
    }
}

/// Maintain a collection of service instances we can remotely interact with.
pub struct Controller {
    services: Arc<Mutex<HashMap<String, Socket>>>,
}

impl Controller {
    /// Start up a new service controller at the given service name.
    /// Asynchronously browse for new services, and remove them if they deregister.
    /// For the moment, panic if anything goes wrong during initialization.
    /// This is acceptable as this action will run once during startup and there's nothing to do
    /// except bail completely if this process fails.
    pub fn new(name: String) -> Self {
        let services = Arc::new(Mutex::new(HashMap::new()));
        let mut ctx = Context::new();

        let services_remote = services.clone();
        // Spawn a new thread to run the tokio event loop.
        // May want to refactor this in the future if we go whole hog on tokio for I/O.
        thread::spawn(move || {
            browse_forever(
                &name,
                |(service, name)| match req_socket(&service.host_target, service.port, &mut ctx) {
                    Ok(socket) => {
                        services_remote.lock().unwrap().insert(name, socket);
                    }
                    Err(e) => {
                        println!("Could not connect to '{}':\n{}", service.host_target, e);
                    }
                },
                |name| {
                    services_remote.lock().unwrap().remove(name);
                },
            );
        });

        Controller { services }
    }

    /// List the services available on this controller.
    pub fn list(&self) -> Vec<String> {
        self.services.lock().unwrap().keys().cloned().collect()
    }

    /// Send a message to one of the services on this controller, returning the response.
    pub fn send(&self, name: &str, msg: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
        let services = self.services.lock().unwrap();
        let socket = match services.get(name) {
            None => bail!(format!("No service named '{}' available.", name)),
            Some(socket) => socket,
        };
        socket.send(msg, 0)?;
        let response = socket.recv_bytes(0)?;
        Ok(response)
    }
}

/// Try to connect a REQ socket at this host and port.
fn req_socket(host: &str, port: u16, ctx: &mut Context) -> Result<Socket, Box<dyn Error>> {
    let addr = format!("tcp://{}:{}", host, port);

    // Connect a REQ socket.
    let socket = ctx.socket(zmq::REQ)?;
    socket.connect(&addr)?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
