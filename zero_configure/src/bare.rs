//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.

use async_dnssd::{
    browse, register_extended, BrowsedFlags, RegisterData, RegisterFlags, ResolveResult,
};
use futures::{Future, Stream};

use simple_error::bail;
use tokio_core::reactor::{Core, Timeout};

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub type StopFn = Box<dyn FnOnce()>;

/// Format a service name into a DNS-SD TCP registration type.
pub fn reg_type(name: &str) -> String {
    format!("_{}._tcp", name)
}

/// Register a vanilla service over DNS-SD.
/// Return a callback that will deregister the service.
pub fn register_service(name: &str, port: u16) -> Result<StopFn, Box<dyn Error>> {
    // FIXME: figure out how to better integrate tokio and deduplicate this code
    let (send_stop, receive_stop) = channel();
    let (send_success, receive_success) = channel();
    let full_name = reg_type(name);

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

        match register_extended(&full_name, port, register_data, &core.handle()) {
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
/// FIXME: there's currently no way to stop the browse thread, it will run until
/// the process terminates even if we drop this struct.
pub struct Browser<S: Send + 'static> {
    service_name: String,
    services: Arc<Mutex<HashMap<String, S>>>,
}

impl<S: Send> Browser<S> {
    /// Start up a new service controller at the given service name.
    /// Asynchronously browse for new services, and remove them if they deregister.
    /// For the moment, panic if anything goes wrong during initialization.
    /// This is acceptable as this action will run once during startup and there's nothing to do
    /// except bail completely if this process fails.
    pub fn new<F>(name: String, open_service: F) -> Self
    where
        F: Fn(&ResolveResult) -> Result<S, Box<dyn Error>> + Send + 'static,
    {
        let services = Arc::new(Mutex::new(HashMap::new()));

        let services_remote = services.clone();
        let service_name = name.clone();
        // Spawn a new thread to run the tokio event loop.
        // May want to refactor this in the future if we go whole hog on tokio for I/O.
        thread::spawn(move || {
            browse_forever(
                &service_name,
                |(service, name)| match open_service(&service) {
                    Ok(service) => {
                        services_remote.lock().unwrap().insert(name, service);
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

        Browser {
            services,
            service_name: name,
        }
    }

    /// List the service instances currently available.
    pub fn list(&self) -> Vec<String> {
        self.services.lock().unwrap().keys().cloned().collect()
    }

    /// Get the name of the service we are browsing.
    pub fn name(&self) -> &str {
        &self.service_name
    }

    /// Borrow a service to perform an action.
    pub fn use_service<A, R>(&self, name: &str, action: A) -> Option<R>
    where
        A: FnOnce(&S) -> R,
    {
        let services = self.services.lock().unwrap();
        services.get(name).map(action)
    }
}

/// Use the current thread to browse for services.
/// Continues browsing forever.
pub fn browse_forever<A, D>(name: &str, mut on_service_appear: A, mut on_service_drop: D)
where
    A: FnMut((ResolveResult, String)),
    D: FnMut(&str),
{
    let registration_type = reg_type(name);
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
                on_service_drop(&event.service_name);
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
        .for_each(|result| {
            on_service_appear(result);
            Ok(())
        });

    core.run(browse_result).unwrap();
}
