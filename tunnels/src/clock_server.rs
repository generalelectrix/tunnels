//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.

use anyhow::Result;

use serde::{Deserialize, Serialize};
use tunnels_lib::number::UnipolarFloat;
use zero_configure::pub_sub::{Config, PublisherService, SubscriberService};

pub use crate::clock_bank::StaticClockBank;

const SERVICE_NAME: &str = "showclocks";
const PORT: u16 = 9090;

/// How the clock stream is carried.
///
/// A bank of clocks is a few hundred bytes and compresses to no advantage, so
/// the stream carries them as they are. The message ceiling is the transport's
/// own, which sits deliberately far above anything the protocol carries: sized
/// to the messages it actually sees, the bound would stand between the show
/// and a message that had merely grown, and a clock stream that stops for that
/// reason gives an operator nothing to work from.
fn config() -> Config {
    Config::default()
}

/// Launch clock publisher service.
pub fn clock_publisher() -> Result<ClockPublisher> {
    PublisherService::new(SERVICE_NAME, PORT, config())
}

/// Launch clock subscriber service.
pub fn clock_subscriber() -> ClockSubscriber {
    SubscriberService::new(SERVICE_NAME.to_string(), config())
}

/// A collection of static clock state data with audio envelope.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct SharedClockData {
    pub clock_bank: StaticClockBank,
    pub audio_envelope: UnipolarFloat,
}

pub type ClockPublisher = PublisherService<SharedClockData>;
pub type ClockSubscriber = SubscriberService<SharedClockData>;

#[cfg(test)]
mod tests {
    use tunnels_lib::number::Phase;

    use crate::clock::StaticClock;
    use crate::clock_bank::{ClockIdx, ClockStore, MAX_CLOCKS};

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
            let bytes = postcard::to_allocvec(&original).unwrap();
            let decoded: StaticClockBank = postcard::from_bytes(&bytes).unwrap();
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
        let bytes = postcard::to_allocvec(&bank(MAX_CLOCKS)).unwrap();
        let decoded: StaticClockBank = postcard::from_bytes(&bytes).unwrap();
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
        let bytes = postcard::to_allocvec(&over).unwrap();
        let decoded: StaticClockBank = postcard::from_bytes(&bytes).unwrap();
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
        let bytes = postcard::to_allocvec(&WireShaped {
            clock_bank: over,
            audio_envelope: envelope,
        })
        .unwrap();
        let decoded: SharedClockData = postcard::from_bytes(&bytes).unwrap();
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
