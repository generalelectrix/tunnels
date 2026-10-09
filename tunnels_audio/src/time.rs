//! Spans of time shared between threads, and strictly positive half-lives.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A [`Duration`] shared between threads, stored as a whole number of
/// nanoseconds. A duration longer than `u64::MAX` nanoseconds (about 584
/// years) is stored as that maximum.
#[derive(Debug)]
pub struct AtomicDuration(AtomicU64);

impl AtomicDuration {
    pub fn new(value: Duration) -> Self {
        Self(AtomicU64::new(Self::to_nanos(value)))
    }

    pub fn get(&self) -> Duration {
        Duration::from_nanos(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, value: Duration) {
        self.0.store(Self::to_nanos(value), Ordering::Relaxed);
    }

    fn to_nanos(value: Duration) -> u64 {
        u64::try_from(value.as_nanos()).unwrap_or(u64::MAX)
    }
}

/// The time over which something halves: a strictly positive [`Duration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HalfLife(Duration);

impl HalfLife {
    /// A half-life of `duration`, or nothing if it is zero.
    pub const fn new(duration: Duration) -> Option<Self> {
        if duration.is_zero() {
            None
        } else {
            Some(Self(duration))
        }
    }

    /// A half-life of `secs` seconds, or nothing if that is not a positive
    /// duration.
    pub fn try_from_secs_f32(secs: f32) -> Option<Self> {
        Duration::try_from_secs_f32(secs).ok().and_then(Self::new)
    }

    /// A half-life of a whole, non-zero number of milliseconds.
    pub const fn from_millis(millis: NonZeroU64) -> Self {
        Self(Duration::from_millis(millis.get()))
    }

    pub const fn get(self) -> Duration {
        self.0
    }
}

/// A [`HalfLife`] shared between threads, stored as an [`AtomicDuration`].
#[derive(Debug)]
pub struct AtomicHalfLife(AtomicDuration);

impl AtomicHalfLife {
    pub fn new(value: HalfLife) -> Self {
        Self(AtomicDuration::new(value.get()))
    }

    pub fn get(&self) -> HalfLife {
        // Only half-lives are stored, and a saturated one is still positive,
        // so the floor of one nanosecond never applies.
        HalfLife(self.0.get().max(Duration::from_nanos(1)))
    }

    pub fn set(&self, value: HalfLife) {
        self.0.set(value.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored duration reads back to the nanosecond; one too long for the
    /// store reads back as the longest it holds.
    #[test]
    fn atomic_duration_round_trips_and_saturates() {
        let atomic = AtomicDuration::new(Duration::new(3, 141_592_653));
        assert_eq!(atomic.get(), Duration::new(3, 141_592_653));
        atomic.set(Duration::ZERO);
        assert_eq!(atomic.get(), Duration::ZERO);
        atomic.set(Duration::MAX);
        assert_eq!(atomic.get(), Duration::from_nanos(u64::MAX));
    }

    /// Zero is not a half-life, nor is a count of seconds that is not a
    /// positive duration; any positive duration is, and an atomic one reads
    /// back what was stored, saturating like [`AtomicDuration`].
    #[test]
    fn half_life_is_strictly_positive() {
        assert_eq!(HalfLife::new(Duration::ZERO), None);
        let shortest = HalfLife::new(Duration::from_nanos(1)).expect("1 ns is positive");
        assert_eq!(shortest.get(), Duration::from_nanos(1));
        let ten_seconds = HalfLife::from_millis(NonZeroU64::new(10_000).expect("non-zero"));
        assert_eq!(ten_seconds.get(), Duration::from_secs(10));
        assert_eq!(HalfLife::try_from_secs_f32(10.0), Some(ten_seconds));
        for not_positive in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                HalfLife::try_from_secs_f32(not_positive),
                None,
                "{not_positive} s"
            );
        }

        let atomic = AtomicHalfLife::new(ten_seconds);
        assert_eq!(atomic.get(), ten_seconds);
        atomic.set(shortest);
        assert_eq!(atomic.get(), shortest);
        let longest = HalfLife::new(Duration::MAX).expect("positive");
        atomic.set(longest);
        assert_eq!(atomic.get().get(), Duration::from_nanos(u64::MAX));
    }
}
