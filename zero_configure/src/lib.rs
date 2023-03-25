//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using 0mq REQ/REP sockets.

use async_dnssd::{browse, register_extended, BrowsedFlags, RegisterData, RegisterFlags};
use futures::{Future, Stream};
use simple_error::bail;
use tokio_core::reactor::{Core, Timeout};

use zmq::{Context, Socket};

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Format a service name into a DNS-SD TCP registration type.
fn reg_type(name: &str) -> String {
    format!("_{}._tcp", name)
}

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

/// Register a vanilla service over DNS-SD.
/// Return a callback that will deregister the service.
pub fn register_service(name: String, port: u16) -> Result<Box<dyn FnOnce()>, Box<dyn Error>> {
    // FIXME: figure out how to better integrate tokio and deduplicate this code
    let (send_stop, receive_stop) = channel();
    let (send_success, receive_success) = channel();

    thread::spawn(move || {
        let core = match Core::new() {
            Err(e) => {
                send_success.send(Err(e)).unwrap();
                return;
            }
            Ok(core) => core,
        };

        // Start advertising this service over DNS-SD.
        let mut register_data = RegisterData::default();
        register_data.flags = RegisterFlags::SHARED;

        match register_extended(&reg_type(&name), port, register_data, &core.handle()) {
            Err(e) => {
                send_success.send(Err(e)).unwrap();
            }
            Ok(_registration) => {
                send_success.send(Ok(())).unwrap();
                receive_stop.recv().unwrap();
            }
        }
    });

    receive_success.recv().unwrap()?;

    Ok(Box::new(move || {
        send_stop.send(()).unwrap();
    }))
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

            let browse_result = browse(&registration_type, &handle)
                .unwrap()
                .filter_map(|event| {
                    // If this service was added, continue processing.
                    if event.flags.contains(BrowsedFlags::ADD) {
                        Some(event)
                    } else {
                        // This service was dropped, remove it from the collection.
                        services_remote.lock().unwrap().remove(&event.service_name);
                        None
                    }
                })
                .and_then(|event| {
                    let resolve_result = event.resolve(&handle);
                    // Attach the service name to the resolve result so we can uniformly use it
                    // to identify a particular client.
                    resolve_result.map(move |res| (res, event.service_name))
                })
                .and_then(|(resolve_stream, service_name)| {
                    // Create a stream that produces None after a timeout, select the two streams,
                    // take items until we produce None due to timeout, then filter them.
                    Ok(Timeout::new(Duration::from_secs(1), &handle)
                        .expect("Couldn't create timeout future.")
                        .into_stream()
                        .map(|_| None)
                        .select(resolve_stream.map(Some))
                        .take_while(|item| Ok(item.is_some()))
                        .filter_map(|x| x)
                        // Tack on the service name.
                        .map(move |resolved| (resolved, service_name.clone())))
                })
                .flatten()
                .for_each(|(service, name)| {
                    // Open a REQ socket to this service and add it to the collection.
                    match req_socket(&service.host_target, service.port, &mut ctx) {
                        Ok(socket) => {
                            services_remote.lock().unwrap().insert(name, socket);
                        }
                        Err(e) => {
                            println!("Could not connect to '{}':\n{}", service.host_target, e);
                        }
                    }
                    Ok(())
                });

            core.run(browse_result).unwrap();
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

        let controller = Controller::new(name);

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
