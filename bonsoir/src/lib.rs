//! bonsoir — UDP multicast heartbeat discovery for LAN services.
//!
//! A simple alternative to DNS-SD/mDNS. Services heartbeat every 2 seconds on a
//! multicast group. Browsers listen for heartbeats and expire services after 6
//! seconds of silence. Drop sends goodbye packets for clean deregistration.

mod browser;
mod multicast;
mod registration;
mod wire;

pub use browser::{BrowseEvent, Browser};
pub use registration::Registration;
pub use wire::ServiceInstance;

use std::time::Duration;

/// Timing configuration for bonsoir discovery.
#[derive(Debug, Clone)]
pub struct Timing {
    /// How often registrations send heartbeat packets.
    pub heartbeat_interval: Duration,
    /// How long a browser waits before considering a service expired.
    pub expiry_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(2),
            expiry_timeout: Duration::from_secs(6),
        }
    }
}

impl Timing {
    /// Fast timing for tests: 50ms heartbeat, 150ms expiry.
    pub fn fast() -> Self {
        Self {
            heartbeat_interval: Duration::from_millis(50),
            expiry_timeout: Duration::from_millis(150),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Initialize env_logger for tests. Calling this multiple times is safe —
    /// `try_init` silently ignores subsequent calls.
    fn init_logging() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn discover_and_expire() {
        init_logging();
        let timing = Timing::fast();

        let reg = Registration::with_timing("bontest", "TestHost", 19995, timing.clone()).unwrap();
        let (_browser, events) = Browser::with_timing("bontest", timing).unwrap();

        let event = events
            .recv_timeout(Duration::from_secs(2))
            .expect("should discover the service");
        match event {
            BrowseEvent::ServiceUp(info) => {
                assert_eq!(info.instance_name, "TestHost");
                assert_eq!(info.port, 19995);
                assert_eq!(info.service_type, "bontest");
            }
            BrowseEvent::ServiceDown(_) => panic!("unexpected ServiceDown"),
        }

        // Drop the registration — should send goodbye packets.
        drop(reg);

        // Goodbye should arrive almost instantly; expiry within 1s.
        let event = events
            .recv_timeout(Duration::from_secs(2))
            .expect("should see ServiceDown");
        match event {
            BrowseEvent::ServiceDown(name) => {
                assert_eq!(name, "TestHost");
            }
            BrowseEvent::ServiceUp(_) => panic!("unexpected ServiceUp after drop"),
        }
    }

    #[test]
    fn query_discovers_existing_service() {
        init_logging();
        let timing = Timing::fast();

        let reg =
            Registration::with_timing("querytest", "QueryHost", 19996, timing.clone()).unwrap();

        // Give the registration a moment to start heartbeating.
        std::thread::sleep(Duration::from_millis(100));

        let (_browser, events) = Browser::with_timing("querytest", timing).unwrap();

        let event = events
            .recv_timeout(Duration::from_secs(2))
            .expect("query should discover existing service");
        match event {
            BrowseEvent::ServiceUp(info) => {
                assert_eq!(info.instance_name, "QueryHost");
            }
            BrowseEvent::ServiceDown(_) => panic!("unexpected ServiceDown"),
        }

        drop(reg);
    }

    #[test]
    fn ignores_other_service_types() {
        init_logging();
        let timing = Timing::fast();

        let _reg =
            Registration::with_timing("othertype", "OtherHost", 19997, timing.clone()).unwrap();
        let (_browser, events) = Browser::with_timing("mytype", timing).unwrap();

        // Should not discover the other service type within several heartbeat cycles.
        let result = events.recv_timeout(Duration::from_secs(1));
        assert!(
            result.is_err(),
            "should not discover a service of a different type"
        );
    }
}
