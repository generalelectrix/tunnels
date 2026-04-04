//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.

use anyhow::Result;

use serde::{Deserialize, Serialize};
use tunnels_lib::number::{Phase, UnipolarFloat};
use zero_configure::pub_sub::{PublisherService, SubscriberService};

use crate::{
    clock::StaticClock,
    clock_bank::{ClockIdx, ClockStore, N_CLOCKS},
};

const SERVICE_NAME: &str = "showclocks";
const PORT: u16 = 9090;

/// Launch clock publisher service.
pub fn clock_publisher() -> Result<ClockPublisher> {
    PublisherService::new(SERVICE_NAME, PORT)
}

/// Launch clock subscriber service.
pub fn clock_subscriber() -> ClockSubscriber {
    SubscriberService::new(SERVICE_NAME.to_string())
}

/// A collection of static clock state data with audio envelope.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct SharedClockData {
    pub clock_bank: StaticClockBank,
    pub audio_envelope: UnipolarFloat,
}

pub type ClockPublisher = PublisherService<SharedClockData>;
pub type ClockSubscriber = SubscriberService<SharedClockData>;

/// A collection of static clock state data, rendered from a ClockBank.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct StaticClockBank(pub [StaticClock; N_CLOCKS]);

impl ClockStore for StaticClockBank {
    fn phase(&self, index: ClockIdx) -> Phase {
        self.get(index).phase
    }

    fn ticks(&self, index: ClockIdx) -> crate::clock::Ticks {
        self.get(index).ticks
    }

    fn submaster_level(&self, index: ClockIdx) -> UnipolarFloat {
        self.get(index).submaster_level
    }

    fn use_audio_size(&self, index: ClockIdx) -> bool {
        self.get(index).use_audio_size
    }
}

impl StaticClockBank {
    fn get(&self, index: ClockIdx) -> &StaticClock {
        &self.0[usize::from(index)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_registers() {
        let stop =
            zero_configure::bare::register_service(SERVICE_NAME, 0).expect("should register");
        stop();
    }

    #[test]
    fn too_long_service_name_rejected() {
        match zero_configure::bare::register_service("this_name_is_too_long", 0) {
            Err(e) => assert!(e.to_string().contains("max is"), "{e}"),
            Ok(_) => panic!("should have rejected name longer than 15 chars"),
        }
    }
}
