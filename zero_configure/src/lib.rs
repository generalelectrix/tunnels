//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using 0mq REQ/REP sockets.
#[macro_use]
extern crate simple_error;
extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;
extern crate zmq;

use async_dnssd::{
    register,
    RegisterFlag,
    Interface,
    browse,
    BrowsedFlag};
use futures::Stream;
use tokio_core::reactor::Core;

use zmq::{Context, Socket};

use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::HashMap;

/// Format a service name into a DNS-SD TCP registration type.
fn reg_type(name: &str) -> String { format!("_{}._tcp", name) }

/// Advertise a service over DNS-SD, using a 0mq REQ/REP socket as the subsequent transport.
/// Pass each message received on the socket to the action callback.  Send the byte buffer returned
/// by the action callback back to the requester.
pub fn run_service<F>(name: &str, port: u16, mut action: F) -> Result<(), Box<Error>>
    where F: FnMut(&[u8]) -> Vec<u8>
{

    let ctx = Context::new();

    // Open the 0mq socket we'll use to service requests.
    let socket = ctx.socket(zmq::REP)?;
    let addr = format!("tcp://*:{}", port);
    socket.bind(&addr)?;

    // Create a tokio core just to run this one future.
    let core = Core::new()?;

    // Start advertising this service over DNS-SD.
    let _registration = register(
        RegisterFlag::Shared.into(),
        Interface::Any,
        None,
        &reg_type(name),
        None,
        None,
        port,
        "".as_bytes(),
        &core.handle())?;

    loop {
        if let Ok(msg) = socket.recv_bytes(0) {
            let response = action(&msg);
            match socket.send(&response, 0) {
                Err(e) => println!("Failed to send response: {}", e),
                _ => (),
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
    pub fn new(name: &str) -> Self {
        let services = Arc::new(Mutex::new(HashMap::new()));
        let mut ctx = Context::new();
        let registration_type = reg_type(name);

        let services_remote = services.clone();
        // Spawn a new thread to run the tokio event loop.
        // May want to refactor this in the future if we go whole hog on tokio for I/O.
        thread::spawn(move || {
            let mut core = Core::new().unwrap();

            let handle = core.handle();

            let browse_result = browse(
                Interface::Any,
                &registration_type,
                None,
                &handle)
                .unwrap()
                .filter_map(|event| {
                    // If this service was added, continue processing.
                    if event.flags & BrowsedFlag::Add {
                        Some(event)
                    } else {
                        // This service was dropped, remove it from the collection.
                        services_remote.lock().unwrap().remove(&event.service_name);
                        None
                    }
                })
                .and_then(|event| event.resolve(&handle))
                .flatten()
                .for_each(|service| {
                    match req_socket(&service.host_target, service.port, &mut ctx) {
                        Ok(socket) => {
                            services_remote.lock().unwrap().insert(service.host_target, socket);
                        },
                        Err(e) => {
                            println!("Could not connect to '{}':\n{}", service.host_target, e);
                        }
                    }
                    Ok(())
                });

            core.run(browse_result).unwrap();
        });

        Controller {
            services,
        }
    }

    /// List the services available on this controller.
    pub fn list(&self) -> Vec<String> {
        self.services.lock().unwrap().keys().map(|name| name.clone()).collect()
    }

    /// Send a message to one of the services on this controller, returning the response.
    pub fn send(&self, name: &str, msg: &[u8]) -> Result<Vec<u8>, Box<Error>> {
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
fn req_socket(host: &str, port: u16, ctx: &mut Context) -> Result<Socket, Box<Error>> {

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
    fn deadbeef() -> Vec<u8> { vec!(0xD, 0xE, 0xA, 0xD, 0xB, 0xE, 0xE, 0xF) }

    /// Return a byte vector containing 0123.
    fn testbytes() -> Vec<u8> { vec!(0, 1, 2, 3) }

    fn sleep(dt: u64) { thread::sleep(Duration::from_millis(dt)) }

    /// Test that we can advertise a single service and successfully connect to it.
    #[test]
    fn test_pair() {

        let name = "test";
        let port = 10000;

        let controller = Controller::new(name);

        // Wait a moment, and assert that we can't see any services.
        sleep(500);

        assert!(controller.list().is_empty());

        // Start up the service; return DEADBEEF as a response.
        thread::spawn(move || {
            run_service(name, port, |buffer| {
                assert_eq!(testbytes(), buffer);
                deadbeef()
            }).unwrap();
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
