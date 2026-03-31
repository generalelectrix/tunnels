//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

pub type StopFn = Box<dyn FnOnce() + Send>;

/// A resolved DNS-SD service instance, containing just the hostname and port.
pub struct ServiceEndpoint {
    pub hostname: String,
    pub port: u16,
}

/// Format a service name into a DNS-SD TCP registration type.
pub fn reg_type(name: &str) -> String {
    format!("_{name}._tcp")
}

/// Format a service name into the fully-qualified type required by mdns-sd.
pub(crate) fn service_type_fq(name: &str) -> String {
    format!("_{name}._tcp.local.")
}

/// Extract the instance name from a fully-qualified service name.
/// e.g. "myinstance._test._tcp.local." with service type "_test._tcp.local." -> "myinstance"
fn instance_name_from_fullname(fullname: &str, service_type: &str) -> String {
    fullname
        .strip_suffix(service_type)
        .unwrap_or(fullname)
        .trim_end_matches('.')
        .to_string()
}

/// Strip the trailing dot from a hostname if present.
/// mdns-sd returns hostnames like "myhost.local." but network APIs expect "myhost.local".
fn strip_trailing_dot(hostname: &str) -> String {
    hostname.strip_suffix('.').unwrap_or(hostname).to_string()
}

/// Get the local hostname in mDNS format (ending with ".local.").
pub(crate) fn mdns_hostname() -> String {
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    // mdns-sd requires hostnames ending in ".local."
    if raw.ends_with(".local.") {
        raw
    } else if raw.ends_with(".local") {
        format!("{raw}.")
    } else {
        format!("{raw}.local.")
    }
}

/// Register a vanilla service over DNS-SD.
/// Return a callback that will deregister the service.
pub fn register_service(name: &str, port: u16) -> Result<StopFn> {
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

    let fullname = service_info.get_fullname().to_string();
    daemon.register(service_info)?;

    Ok(Box::new(move || {
        let _ = daemon.unregister(&fullname);
        let _ = daemon.shutdown();
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
        F: Fn(&ServiceEndpoint) -> Result<S> + Send + 'static,
    {
        let services = Arc::new(Mutex::new(HashMap::new()));

        let services_remote = services.clone();
        let service_name = name.clone();
        // Spawn a new thread to run the browse event loop.
        thread::spawn(move || {
            browse_forever(
                &service_name,
                |(endpoint, name)| match open_service(&endpoint) {
                    Ok(service) => {
                        services_remote.lock().unwrap().insert(name, service);
                    }
                    Err(e) => {
                        println!("Could not connect to '{}:{}':\n{}", endpoint.hostname, endpoint.port, e);
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
    A: FnMut((ServiceEndpoint, String)),
    D: FnMut(&str),
{
    let service_type = service_type_fq(name);
    let daemon = ServiceDaemon::new().expect("Failed to create mDNS daemon");
    let receiver = daemon
        .browse(&service_type)
        .expect("Failed to start mDNS browse");

    loop {
        match receiver.recv() {
            Ok(event) => match event {
                ServiceEvent::ServiceResolved(info) => {
                    let instance_name =
                        instance_name_from_fullname(info.get_fullname(), &service_type);
                    // Prefer a resolved IPv4 address over the mDNS hostname, since the
                    // hostname (e.g. "myhost.local") may not be resolvable by the
                    // system DNS resolver.
                    let host = info
                        .get_addresses_v4()
                        .into_iter()
                        .next()
                        .map(|addr| addr.to_string())
                        .unwrap_or_else(|| strip_trailing_dot(info.get_hostname()));
                    let endpoint = ServiceEndpoint {
                        hostname: host,
                        port: info.get_port(),
                    };
                    on_service_appear((endpoint, instance_name));
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    let instance_name =
                        instance_name_from_fullname(&fullname, &service_type);
                    on_service_drop(&instance_name);
                }
                _ => {}
            },
            Err(_) => {
                // The daemon has shut down; stop browsing.
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_reg_type() {
        assert_eq!(reg_type("foo"), "_foo._tcp");
        assert_eq!(reg_type("myservice"), "_myservice._tcp");
    }

    #[test]
    fn test_service_type_fq() {
        assert_eq!(service_type_fq("foo"), "_foo._tcp.local.");
    }

    #[test]
    fn test_instance_name_extraction() {
        assert_eq!(
            instance_name_from_fullname("mybox._test._tcp.local.", "_test._tcp.local."),
            "mybox"
        );
        assert_eq!(
            instance_name_from_fullname("a.b.c._svc._tcp.local.", "_svc._tcp.local."),
            "a.b.c"
        );
    }

    #[test]
    fn test_strip_trailing_dot() {
        assert_eq!(strip_trailing_dot("myhost.local."), "myhost.local");
        assert_eq!(strip_trailing_dot("myhost.local"), "myhost.local");
    }

    #[test]
    fn test_register_and_stop() {
        let stop = register_service("regtest", 19990).unwrap();
        stop();
    }

    #[test]
    fn test_register_and_browse() {
        let stop = register_service("browsetest", 19991).unwrap();
        // Give the registration a moment to propagate.
        thread::sleep(Duration::from_millis(500));

        let browser: Browser<()> = Browser::new("browsetest".to_string(), |_| Ok(()));
        // Give the browser a moment to discover.
        thread::sleep(Duration::from_secs(3));

        let services = browser.list();
        assert!(!services.is_empty(), "Browser should have discovered at least one service");
        stop();
    }
}
