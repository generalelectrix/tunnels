//! Advertise a service over DNS-SD.  Browse for and agglomerate instances of this service.

use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceEvent, ServiceInfo};

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Maximum service name length per RFC 6763.
/// mdns-sd only validates this asynchronously on its daemon thread, so we check
/// eagerly at registration time to surface errors immediately.
const SERVICE_NAME_LEN_MAX: usize = 15;

pub type StopFn = Box<dyn FnOnce() + Send>;

/// Maximum length of a single DNS label (RFC 1035).
const MAX_DNS_LABEL_LEN: usize = 63;

/// Truncate a string to fit within a DNS label.
fn truncate_to_dns_label(s: &str) -> &str {
    &s[..s.len().min(MAX_DNS_LABEL_LEN)]
}

/// A resolved DNS-SD service instance: the address and port to connect to.
pub(crate) struct ServiceEndpoint {
    pub(crate) hostname: String,
    pub(crate) port: u16,
}

/// Format a service name into the fully-qualified mDNS service type (e.g. `"_foo._tcp.local."`).
///
/// mdns-sd requires the `.local.` suffix on all service types.
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

/// Get the machine's display name for use as a DNS-SD instance name.
/// On macOS, uses the Computer Name (e.g. "Bore A") to match Apple's native DNS-SD behavior.
/// Falls back to the short hostname.
fn machine_hostname() -> String {
    // Try macOS Computer Name first.
    if let Ok(output) = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
    {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return truncate_to_dns_label(&name).to_string();
            }
        }
    }
    // Fall back to short hostname.
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let short = raw.split('.').next().unwrap_or(&raw);
    truncate_to_dns_label(short).to_string()
}

/// Build a service-scoped mDNS hostname that won't collide with the system hostname.
///
/// macOS's built-in mDNS responder already advertises A/AAAA records for the system
/// hostname (e.g. "bore-b.local."). The mdns-sd library also publishes A/AAAA records
/// for whatever hostname we give it, so using the bare system hostname causes a
/// collision — macOS detects the duplicate and renames itself (e.g. "bore-b-2.local").
///
/// By appending the service name to the hostname stem (e.g. "bore-b-tunnelbootstrap.local."),
/// we avoid the collision while still getting correct IP resolution via enable_addr_auto().
fn mdns_service_hostname(service_name: &str) -> String {
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    let stem = raw
        .strip_suffix(".local.")
        .or_else(|| raw.strip_suffix(".local"))
        .unwrap_or(&raw);
    // Truncate the stem so "{stem}-{service_name}" fits in a DNS label.
    let max_stem = MAX_DNS_LABEL_LEN - 1 - service_name.len();
    let stem = &stem[..stem.len().min(max_stem)];
    format!("{stem}-{service_name}.local.")
}

/// Create a `ServiceDaemon` and register a service with automatic address resolution.
/// Uses the machine's hostname as the instance name, matching the behavior of Apple's
/// native DNS-SD API (async-dnssd).
/// Returns the daemon (which must be kept alive to maintain the registration) and
/// the fullname needed for later unregistration.
pub(crate) fn create_and_register(name: &str, port: u16) -> Result<(ServiceDaemon, String)> {
    if name.len() > SERVICE_NAME_LEN_MAX {
        bail!(
            "Service name {:?} is {} bytes, max is {}",
            name,
            name.len(),
            SERVICE_NAME_LEN_MAX,
        );
    }

    let service_type = service_type_fq(name);
    let daemon = ServiceDaemon::new()?;

    let hostname = mdns_service_hostname(name);
    // Use the raw hostname (without .local. suffix) as the instance name,
    // so each machine advertises with its own name rather than the service type.
    let instance_name = machine_hostname();

    let service_info = ServiceInfo::new(
        &service_type,
        &instance_name,
        &hostname,
        "",
        port,
        None::<HashMap<String, String>>,
    )?
    .enable_addr_auto();

    let fullname = service_info.get_fullname().to_string();
    daemon.register(service_info)?;

    Ok((daemon, fullname))
}

/// Register a vanilla service over DNS-SD.
/// Return a callback that will deregister the service.
pub fn register_service(name: &str, port: u16) -> Result<StopFn> {
    let (daemon, fullname) = create_and_register(name, port)?;

    Ok(Box::new(move || {
        let _ = daemon.unregister(&fullname);
        let _ = daemon.shutdown();
    }))
}

/// Signal sent to the browse thread to control the daemon lifecycle.
/// Signal sent to the browse thread to tear down the current mDNS daemon
/// and start fresh. The retry loop will create a new daemon immediately.
pub(crate) enum BrowseControl {
    Restart,
}

/// Maintain a collection of service instances we can remotely interact with.
/// FIXME: there's currently no way to stop the browse thread, it will run until
/// the process terminates even if we drop this struct.
pub(crate) struct Browser<S: Send + 'static> {
    service_name: String,
    services: Arc<Mutex<HashMap<String, S>>>,
    control_tx: Sender<BrowseControl>,
}

impl<S: Send> Browser<S> {
    /// Start up a new service controller at the given service name.
    /// Asynchronously browse for new services, and remove them if they deregister.
    /// For the moment, panic if anything goes wrong during initialization.
    /// This is acceptable as this action will run once during startup and there's nothing to do
    /// except bail completely if this process fails.
    pub(crate) fn new<F>(name: String, open_service: F) -> Self
    where
        F: Fn(&ServiceEndpoint) -> Result<S> + Send + 'static,
    {
        let (control_tx, control_rx) = std::sync::mpsc::channel();
        Self::new_with_control(name, open_service, control_tx, control_rx)
    }

    /// Like `new`, but accepts pre-built control channels.
    /// Used by tests to send Shutdown signals.
    pub(crate) fn new_with_control<F>(
        name: String,
        open_service: F,
        control_tx: Sender<BrowseControl>,
        control_rx: Receiver<BrowseControl>,
    ) -> Self
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
                        log::warn!(
                            "[dnssd] Could not connect to '{}:{}': {}",
                            endpoint.hostname,
                            endpoint.port,
                            e
                        );
                    }
                },
                |name| {
                    services_remote.lock().unwrap().remove(name);
                },
                control_rx,
            );
        });

        Browser {
            services,
            service_name: name,
            control_tx,
        }
    }

    /// List the service instances currently available.
    pub(crate) fn list(&self) -> Vec<String> {
        self.services.lock().unwrap().keys().cloned().collect()
    }

    /// Get the name of the service we are browsing.
    pub(crate) fn name(&self) -> &str {
        &self.service_name
    }

    /// Force-restart the mDNS daemon, clearing stale services and re-browsing
    /// from scratch. Use this when the network environment has changed and
    /// passive discovery isn't recovering.
    pub(crate) fn refresh(&self) {
        self.services.lock().unwrap().clear();
        let _ = self.control_tx.send(BrowseControl::Restart);
    }

    /// Borrow a service to perform an action.
    pub(crate) fn use_service<A, R>(&self, name: &str, action: A) -> Option<R>
    where
        A: FnOnce(&S) -> R,
    {
        let services = self.services.lock().unwrap();
        services.get(name).map(action)
    }
}

/// Block the current thread browsing for services. If the daemon dies or a
/// refresh is requested, tear down and restart with a fresh daemon.
fn browse_forever<A, D>(
    name: &str,
    mut on_service_appear: A,
    mut on_service_drop: D,
    control_rx: Receiver<BrowseControl>,
) where
    A: FnMut((ServiceEndpoint, String)),
    D: FnMut(&str),
{
    let service_type = service_type_fq(name);
    // Share the control receiver across loop iterations via Arc<Mutex>.
    let control_rx = Arc::new(Mutex::new(control_rx));

    loop {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                log::error!("[dnssd] Failed to create mDNS daemon for '{name}': {e}");
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        // Hook up the daemon's monitor channel to log network events.
        match daemon.monitor() {
            Ok(monitor_rx) => {
                thread::spawn(move || {
                    while let Ok(event) = monitor_rx.recv() {
                        match event {
                            DaemonEvent::IpAdd(addr) => {
                                log::info!("[dnssd] Interface added: {addr}");
                            }
                            DaemonEvent::IpDel(addr) => {
                                log::info!("[dnssd] Interface removed: {addr}");
                            }
                            DaemonEvent::Error(e) => {
                                log::error!("[dnssd] Daemon error: {e}");
                            }
                            other => {
                                log::debug!("[dnssd] Daemon event: {other:?}");
                            }
                        }
                    }
                });
            }
            Err(e) => {
                log::warn!("[dnssd] Could not attach daemon monitor: {e}");
            }
        }

        let receiver = match daemon.browse(&service_type) {
            Ok(r) => r,
            Err(e) => {
                log::error!("[dnssd] Failed to start browse for '{name}': {e}");
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        // Spawn a helper thread that waits for control signals and acts on the daemon.
        let control_rx_clone = control_rx.clone();
        thread::spawn(move || {
            if let Ok(rx) = control_rx_clone.lock() {
                if rx.recv().is_ok() {
                    let _ = daemon.shutdown();
                }
            }
        });

        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let instance_name =
                        instance_name_from_fullname(info.get_fullname(), &service_type);
                    let host = info
                        .get_addresses_v4()
                        .into_iter()
                        .next()
                        .map(|addr| addr.to_string())
                        .unwrap_or_else(|| strip_trailing_dot(info.get_hostname()));
                    log::info!(
                        "[dnssd] Resolved '{instance_name}' at {host}:{}",
                        info.get_port()
                    );
                    let endpoint = ServiceEndpoint {
                        hostname: host,
                        port: info.get_port(),
                    };
                    on_service_appear((endpoint, instance_name));
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    let instance_name = instance_name_from_fullname(&fullname, &service_type);
                    log::info!("[dnssd] Service removed: '{instance_name}'");
                    on_service_drop(&instance_name);
                }
                ServiceEvent::SearchStarted(detail) => {
                    log::info!("[dnssd] Browse started: {detail}");
                }
                ServiceEvent::ServiceFound(ty, fullname) => {
                    log::info!("[dnssd] Service found: {fullname} ({ty}), awaiting resolve...");
                }
                ServiceEvent::SearchStopped(ty) => {
                    log::warn!("[dnssd] Browse stopped for {ty}");
                }
                _ => {}
            }
        }

        // Daemon channel closed — restart with a fresh daemon after a brief backoff.
        log::warn!("[dnssd] Browse for '{name}' interrupted, restarting in 2s...");
        thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

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
    fn name_at_max_length_accepted() {
        // 15 chars, right at the RFC 6763 limit.
        let stop = register_service("exactly15chrsss", 0).unwrap();
        stop();
    }

    #[test]
    fn name_too_long_rejected() {
        // 16 chars, one over the limit.
        match register_service("toolongservicenm", 0) {
            Err(e) => assert!(e.to_string().contains("max is"), "{e}"),
            Ok(_) => panic!("should have rejected name longer than 15 chars"),
        }
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
        assert!(
            !services.is_empty(),
            "Browser should have discovered at least one service"
        );
        stop();
    }

    #[test]
    fn browser_recovers_after_daemon_death() {
        // Track the total number of ServiceResolved events the browser sees.
        let resolve_count = Arc::new(AtomicUsize::new(0));
        let resolve_counter = resolve_count.clone();

        // 1. Register a service and create a browser with a kill channel.
        let stop_a = register_service("recovtest", 19993).unwrap();
        thread::sleep(Duration::from_millis(500));

        let (control_tx, control_rx) = std::sync::mpsc::channel();
        let browser: Browser<()> = Browser::new_with_control(
            "recovtest".to_string(),
            move |_| {
                resolve_counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            control_tx,
            control_rx,
        );

        // Wait for the browser to discover service A.
        thread::sleep(Duration::from_secs(3));
        let count_before_kill = resolve_count.load(Ordering::SeqCst);
        assert!(
            count_before_kill > 0,
            "browser should have resolved at least one service before daemon death"
        );

        // 2. Kill the browser's daemon to simulate daemon death.
        browser.control_tx.send(BrowseControl::Restart).unwrap();
        // Wait for the daemon to shut down and the browse loop to exit.
        thread::sleep(Duration::from_secs(1));

        // 3. Deregister A, register a new service B on a different port.
        stop_a();
        thread::sleep(Duration::from_millis(500));
        let stop_b = register_service("recovtest", 19994).unwrap();

        // 4. Wait long enough for recovery + rediscovery.
        thread::sleep(Duration::from_secs(8));

        // 5. Assert the browser resolved a NEW service after daemon death.
        let count_after_recovery = resolve_count.load(Ordering::SeqCst);
        assert!(
            count_after_recovery > count_before_kill,
            "browser should have resolved new services after recovery \
             (before={count_before_kill}, after={count_after_recovery})"
        );

        stop_b();
    }
}
