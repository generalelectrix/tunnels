//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.

use std::fmt;

use anyhow::Result;

use arrayvec::ArrayVec;
use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use tunnels_lib::number::{Phase, UnipolarFloat};
use zero_configure::pub_sub::{PublisherService, SubscriberService};

use crate::{
    clock::StaticClock,
    clock_bank::{ClockIdx, ClockStore, MAX_CLOCKS},
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
#[derive(Serialize, Default, Debug, Clone)]
pub struct StaticClockBank(pub ArrayVec<StaticClock, MAX_CLOCKS>);

impl<'de> Deserialize<'de> for StaticClockBank {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BankVisitor;

        impl<'de> Visitor<'de> for BankVisitor {
            type Value = StaticClockBank;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a sequence of clock states")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<StaticClockBank, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut clocks = ArrayVec::new();
                // Keep the first MAX_CLOCKS clocks and discard any beyond the
                // capacity ceiling, so a peer with a higher ceiling degrades to a
                // usable bank instead of dropping the frame. The loop still drains
                // the whole sequence to keep the rest of the message aligned.
                while let Some(clock) = seq.next_element::<StaticClock>()? {
                    let _ = clocks.try_push(clock);
                }
                Ok(StaticClockBank(clocks))
            }
        }

        deserializer.deserialize_seq(BankVisitor)
    }
}

impl ClockStore for StaticClockBank {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn phase(&self, index: ClockIdx) -> Option<Phase> {
        self.get(index).map(|c| c.phase)
    }

    fn ticks(&self, index: ClockIdx) -> Option<crate::clock::Ticks> {
        self.get(index).map(|c| c.ticks)
    }

    fn submaster_level(&self, index: ClockIdx) -> Option<UnipolarFloat> {
        self.get(index).map(|c| c.submaster_level)
    }

    fn use_audio_size(&self, index: ClockIdx) -> Option<bool> {
        self.get(index).map(|c| c.use_audio_size)
    }
}

impl StaticClockBank {
    /// Return the clock at `index`, or `None` if the index is out of range.
    fn get(&self, index: ClockIdx) -> Option<&StaticClock> {
        self.0.get(index.0)
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

    /// Build a bank of `n` clocks whose `ticks` encode their index, so ordering
    /// and length survive a round trip observably.
    fn bank(n: usize) -> StaticClockBank {
        StaticClockBank(
            (0..n)
                .map(|i| StaticClock {
                    phase: Phase::ZERO,
                    ticks: i as crate::clock::Ticks,
                    submaster_level: UnipolarFloat::ZERO,
                    use_audio_size: false,
                })
                .collect(),
        )
    }

    #[test]
    fn wire_round_trips_at_various_lengths() {
        for n in [0usize, 4, 8, MAX_CLOCKS] {
            let original = bank(n);
            let bytes = rmp_serde::to_vec(&original).unwrap();
            let decoded: StaticClockBank = rmp_serde::from_slice(&bytes).unwrap();
            assert_eq!(decoded.0.len(), n, "length preserved for {n} clocks");
            let ticks: Vec<i64> = decoded.0.iter().map(|c| c.ticks).collect();
            assert_eq!(
                ticks,
                (0..n as i64).collect::<Vec<_>>(),
                "clock order preserved for {n} clocks"
            );
        }
    }

    #[test]
    fn wire_decode_truncates_over_capacity() {
        // At capacity decodes fully.
        let bytes = rmp_serde::to_vec(&bank(MAX_CLOCKS)).unwrap();
        let decoded: StaticClockBank = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.0.len(), MAX_CLOCKS);

        // Beyond capacity: the excess clocks are dropped, keeping the first
        // MAX_CLOCKS in order, rather than erroring or panicking.
        let over: Vec<StaticClock> = (0..MAX_CLOCKS + 3)
            .map(|i| StaticClock {
                phase: Phase::ZERO,
                ticks: i as crate::clock::Ticks,
                submaster_level: UnipolarFloat::ZERO,
                use_audio_size: false,
            })
            .collect();
        let bytes = rmp_serde::to_vec(&over).unwrap();
        let decoded: StaticClockBank = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.0.len(), MAX_CLOCKS, "truncated to capacity");
        let ticks: Vec<i64> = decoded.0.iter().map(|c| c.ticks).collect();
        assert_eq!(
            ticks,
            (0..MAX_CLOCKS as i64).collect::<Vec<_>>(),
            "kept the first MAX_CLOCKS in order"
        );

        // Draining the overflow keeps the rest of the message aligned: an
        // over-length clock_bank in a SharedClockData-shaped message still
        // decodes the trailing audio_envelope correctly.
        #[derive(Serialize)]
        struct WireShaped {
            clock_bank: Vec<StaticClock>,
            audio_envelope: UnipolarFloat,
        }
        let envelope = UnipolarFloat::new(0.75);
        let bytes = rmp_serde::to_vec(&WireShaped {
            clock_bank: over,
            audio_envelope: envelope,
        })
        .unwrap();
        let decoded: SharedClockData = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.clock_bank.0.len(), MAX_CLOCKS);
        assert_eq!(
            decoded.audio_envelope, envelope,
            "trailing field stayed aligned after draining overflow"
        );
    }

    #[test]
    fn out_of_range_reads_return_none() {
        let b = bank(4);
        let missing = ClockIdx(9);
        assert!(b.phase(missing).is_none());
        assert!(b.ticks(missing).is_none());
        assert!(b.submaster_level(missing).is_none());
        assert!(b.use_audio_size(missing).is_none());
        assert_eq!(b.len(), 4);
        // In-range reads return Some.
        assert!(b.phase(ClockIdx(0)).is_some());
    }
}
