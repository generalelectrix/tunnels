//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.
//! FIXME: would be nice to clean up deserialization to avoid so many allocations.

use anyhow::Result;

use serde::{Deserialize, Serialize};
use tunnels_lib::number::{Phase, UnipolarFloat};
use zero_configure::pub_sub::{PublisherService, SubscriberService};
use zmq::Context;

use crate::{
    clock::StaticClock,
    clock_bank::{ClockIdx, ClockStore, N_CLOCKS},
};

const SERVICE_NAME: &str = "global_show_clocks";
const PORT: u16 = 9090;

/// Launch clock publisher service.
pub fn clock_publisher(ctx: &Context) -> Result<ClockPublisher> {
    PublisherService::new(ctx, SERVICE_NAME, PORT)
}

/// Launch clock subscriber service.
pub fn clock_subscriber(ctx: Context) -> ClockSubscriber {
    SubscriberService::new(ctx, SERVICE_NAME.to_string())
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
