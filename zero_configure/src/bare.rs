//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.
//! Interact with one or more instances of this service, using 0mq REQ/REP sockets.

use async_dnssd::{
    browse, register_extended, BrowsedFlags, RegisterData, RegisterFlags, ResolveResult,
};
use futures::{Future, Stream};

use tokio_core::reactor::{Core, Timeout};

use std::error::Error;
use std::sync::mpsc::channel;
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
