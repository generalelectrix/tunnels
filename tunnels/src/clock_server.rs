//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.
//! FIXME: would be nice to clean up deserialization to avoid so many allocations.

use tunnels_lib::number::{Phase, UnipolarFloat};
use zero_configure::pub_sub::PublisherService;

use crate::{
    clock::StaticClock,
    clock_bank::{ClockBank, ClockIdx, ClockStore, N_CLOCKS},
};

pub type ClockPublisher = PublisherService<ClockBank>;

/// A collection of static clock state data, rendered from a ClockBank.
pub struct StaticClockBank([StaticClock; N_CLOCKS]);

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
    fn from_clock_bank(bank: &ClockBank) -> Self {
        Self(bank.as_static())
    }

    fn get(&self, index: ClockIdx) -> &StaticClock {
        &self.0[usize::from(index)]
    }
}
