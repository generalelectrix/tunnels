//! bonsoir — UDP multicast heartbeat discovery for LAN services.
//!
//! A simple alternative to DNS-SD/mDNS. Services heartbeat every 2 seconds on a
//! multicast group. Browsers listen for heartbeats and expire services after 6
//! seconds of silence. Drop sends goodbye packets for clean deregistration.

mod browser;
mod multicast;
mod registration;
mod wire;

pub use browser::{Browser, BrowseEvent};
pub use registration::Registration;
pub use wire::ServiceInstance;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn discover_and_expire() {
        // Register a service.
        let reg = Registration::new("bontest", "TestHost", 19995).unwrap();

        // Start browsing.
        let (browser, events) = Browser::new("bontest").unwrap();

        // Wait for discovery (should happen within 3 seconds).
        let event = events.recv_timeout(Duration::from_secs(3)).expect(
            "should discover the service within 3 seconds",
        );
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

        // Wait for the service to disappear (goodbye or expiry, within 8 seconds).
        let event = events.recv_timeout(Duration::from_secs(8)).expect(
            "should see ServiceDown within 8 seconds",
        );
        match event {
            BrowseEvent::ServiceDown(name) => {
                assert_eq!(name, "TestHost");
            }
            BrowseEvent::ServiceUp(_) => panic!("unexpected ServiceUp after drop"),
        }

        drop(browser);
    }

    #[test]
    fn query_discovers_existing_service() {
        // Register first, then browse — the query mechanism should find it.
        let reg = Registration::new("querytest", "QueryHost", 19996).unwrap();

        // Give the registration a moment to start heartbeating.
        std::thread::sleep(Duration::from_millis(500));

        // Now start browsing — the initial query should get an immediate response.
        let (_browser, events) = Browser::new("querytest").unwrap();

        let event = events.recv_timeout(Duration::from_secs(3)).expect(
            "query should discover existing service within 3 seconds",
        );
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
        let _reg = Registration::new("othertype", "OtherHost", 19997).unwrap();

        let (_browser, events) = Browser::new("mytype").unwrap();

        // Should not discover the other service type.
        let result = events.recv_timeout(Duration::from_secs(4));
        assert!(
            result.is_err(),
            "should not discover a service of a different type"
        );
    }
}
