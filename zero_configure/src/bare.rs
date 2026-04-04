//! Advertise a service via bonsoir. Browse for and agglomerate instances of this service.

use bonsoir::{BrowseEvent, Registration};

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Maximum service name length (matching the original DNS-SD limit).
const SERVICE_NAME_LEN_MAX: usize = 15;

pub type StopFn = Box<dyn FnOnce() + Send>;

/// Maximum length of an instance name.
const MAX_INSTANCE_NAME_LEN: usize = 63;

/// Truncate a string to fit within the instance name limit.
fn truncate_to_label(s: &str) -> &str {
    &s[..s.len().min(MAX_INSTANCE_NAME_LEN)]
}

/// A resolved service instance: the address and port to connect to.
pub(crate) struct ServiceEndpoint {
    pub(crate) hostname: String,
    pub(crate) port: u16,
}

/// Get the machine's display name for use as a service instance name.
/// On macOS, uses the Computer Name (e.g. "Bore A").
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
                return truncate_to_label(&name).to_string();
            }
        }
    }
    // Fall back to short hostname.
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let short = raw.split('.').next().unwrap_or(&raw);
    truncate_to_label(short).to_string()
}

/// Register a service via bonsoir heartbeats.
/// Returns the Registration handle (which must be kept alive) and the instance name.
pub(crate) fn create_and_register(name: &str, port: u16) -> Result<(Registration, String)> {
    if name.len() > SERVICE_NAME_LEN_MAX {
        bail!(
            "Service name {:?} is {} bytes, max is {}",
            name,
            name.len(),
            SERVICE_NAME_LEN_MAX,
        );
    }

    let instance_name = machine_hostname();
    let registration = Registration::new(name, &instance_name, port)?;
    Ok((registration, instance_name))
}

/// Register a service. Return a callback that will deregister it.
pub fn register_service(name: &str, port: u16) -> Result<StopFn> {
    let (registration, _instance_name) = create_and_register(name, port)?;
    // Drop sends goodbye packets.
    Ok(Box::new(move || drop(registration)))
}

/// Signal sent to the browse thread to restart discovery.
pub(crate) enum BrowseControl {
    Restart,
}

/// Maintain a collection of service instances we can remotely interact with.
pub(crate) struct Browser<S: Send + 'static> {
    service_name: String,
    services: Arc<Mutex<HashMap<String, S>>>,
    control_tx: Sender<BrowseControl>,
}

impl<S: Send> Browser<S> {
    /// Start up a new service controller at the given service name.
    /// Asynchronously browse for new services, and remove them when they expire.
    pub(crate) fn new<F>(name: String, open_service: F) -> Self
    where
        F: Fn(&ServiceEndpoint) -> Result<S> + Send + 'static,
    {
        let (control_tx, control_rx) = std::sync::mpsc::channel();
        Self::new_with_control(name, open_service, control_tx, control_rx)
    }

    /// Like `new`, but accepts pre-built control channels (for testing).
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
        thread::spawn(move || {
            browse_forever(
                &service_name,
                |(endpoint, name)| match open_service(&endpoint) {
                    Ok(service) => {
                        services_remote.lock().unwrap().insert(name, service);
                    }
                    Err(e) => {
                        log::warn!(
                            "[bonsoir] Could not connect to '{}:{}': {}",
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

    /// Force-restart discovery, clearing stale services and re-browsing
    /// from scratch.
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

/// Block the current thread browsing for services. If the browser is restarted
/// (via control channel), tear down and recreate.
fn browse_forever<A, D>(
    name: &str,
    mut on_service_appear: A,
    mut on_service_drop: D,
    control_rx: Receiver<BrowseControl>,
) where
    A: FnMut((ServiceEndpoint, String)),
    D: FnMut(&str),
{
    loop {
        let (_browser, event_rx) = match bonsoir::Browser::new(name) {
            Ok(b) => b,
            Err(e) => {
                log::error!("[bonsoir] Failed to create browser for '{name}': {e}");
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        loop {
            // Check for control signals (non-blocking).
            if control_rx.try_recv().is_ok() {
                log::info!("[bonsoir] Restart requested, recreating browser for '{name}'");
                break;
            }

            match event_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(BrowseEvent::ServiceUp(info)) => {
                    let endpoint = ServiceEndpoint {
                        hostname: info.address.to_string(),
                        port: info.port,
                    };
                    on_service_appear((endpoint, info.instance_name));
                }
                Ok(BrowseEvent::ServiceDown(instance_name)) => {
                    on_service_drop(&instance_name);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    log::warn!("[bonsoir] Browser channel disconnected for '{name}'");
                    break;
                }
            }
        }

        // Brief backoff before restarting.
        thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn test_register_and_stop() {
        let stop = register_service("regtest", 19990).unwrap();
        stop();
    }

    #[test]
    fn name_at_max_length_accepted() {
        let stop = register_service("exactly15chrsss", 0).unwrap();
        stop();
    }

    #[test]
    fn name_too_long_rejected() {
        match register_service("toolongservicenm", 0) {
            Err(e) => assert!(e.to_string().contains("max is"), "{e}"),
            Ok(_) => panic!("should have rejected name longer than 15 chars"),
        }
    }

    #[test]
    fn test_register_and_browse() {
        let stop = register_service("browsetest", 19991).unwrap();
        thread::sleep(Duration::from_millis(500));

        let browser: Browser<()> = Browser::new("browsetest".to_string(), |_| Ok(()));
        thread::sleep(Duration::from_secs(3));

        let services = browser.list();
        assert!(
            !services.is_empty(),
            "Browser should have discovered at least one service"
        );
        stop();
    }

    #[test]
    fn browser_recovers_after_restart() {
        let resolve_count = Arc::new(AtomicUsize::new(0));
        let resolve_counter = resolve_count.clone();

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

        thread::sleep(Duration::from_secs(3));
        let count_before = resolve_count.load(Ordering::SeqCst);
        assert!(
            count_before > 0,
            "browser should have resolved at least one service"
        );

        // Restart the browser.
        browser.control_tx.send(BrowseControl::Restart).unwrap();
        thread::sleep(Duration::from_secs(1));

        // Deregister A, register B.
        stop_a();
        thread::sleep(Duration::from_millis(500));
        let stop_b = register_service("recovtest", 19994).unwrap();

        thread::sleep(Duration::from_secs(5));

        let count_after = resolve_count.load(Ordering::SeqCst);
        assert!(
            count_after > count_before,
            "browser should have resolved new services after restart \
             (before={count_before}, after={count_after})"
        );

        stop_b();
    }
}
