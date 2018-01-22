extern crate async_dnssd;
extern crate tokio_core;
extern crate futures;
extern crate zmq;

use async_dnssd::{
    register,
    RegisterFlag,
    Interface,
    Registration,
    browse,
    BrowsedFlag};
use futures::{Future, Stream};
use tokio_core::reactor::{Core, Handle};

use zmq::{Context, Socket};

use std::net::ToSocketAddrs;
use std::net::IpAddr;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::HashMap;

/// Format a service name into a DNS-SD TCP registration type.
fn reg_type(name: &str) -> String { format!("_{}._tcp", name) }

/// Advertise a service over DNS-SD, using a 0mq REQ/REP socket as the subsequent transport.
pub struct Service {
    registration: Registration,
    socket: Socket,
}

impl Service {
    /// Start advertising this service.
    pub fn start(name: &str, port: u16) -> Result<Self, Box<Error>> {

        let ctx = Context::new();

        // Open the 0mq socket we'll use to service requests.
        let socket = ctx.socket(zmq::REP)?;
        let addr = format!("tcp://*:{}", port);
        socket.connect(&addr)?;

        // Create a tokio core just to run this one future.
        let core = Core::new()?;

        // Start advertising this service over DNS-SD.
        let (registration, _) = register(
            RegisterFlag::Shared.into(),
            Interface::Any,
            None,
            &reg_type(name),
            None,
            None,
            port,
            "".as_bytes(),
            &core.handle())?
            .wait()?;

        Ok(Service {
            registration,
            socket,
        })
    }

    /// Run the service synchronously, servicing requests using the provided callback.
    pub fn run(&mut self, action: fn(&[u8]) -> &[u8]) {

        loop {
            if let Ok(msg) = self.socket.recv_bytes(0) {
                let response = action(&msg);
                match self.socket.send(response, 0) {
                    Err(e) => println!("Failed to send response: {}", e),
                    _ => (),
                }
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
    /// Asynchronously browses for new services, and removes them if they deregister.
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
                    println!("Adding service: {:?}", service);

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
}

/// Try to connect a REQ socket at this host and port.
fn req_socket(host: &str, port: u16, ctx: &mut Context) -> Result<Socket, Box<Error>> {
    let addr = format!("tcp://{}:{}", host, port);

    // Bind a REQ socket.
    let socket = ctx.socket(zmq::REQ)?;
    socket.connect(&addr)?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
