//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.
//! FIXME: would be nice to clean up deserialization to avoid so many allocations.

use std::error::Error;

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
pub fn clock_publisher(ctx: &Context) -> Result<ClockPublisher, Box<dyn Error>> {
    PublisherService::new(ctx, SERVICE_NAME, PORT)
}

/// Launch clock subscriber service.
pub fn clock_subscriber(ctx: Context) -> ClockSubscriber {
    SubscriberService::new(ctx, SERVICE_NAME.to_string())
}

pub type ClockPublisher = PublisherService<StaticClockBank>;
pub type ClockSubscriber = SubscriberService<StaticClockBank>;

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
