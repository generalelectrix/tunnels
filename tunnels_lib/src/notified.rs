//! Shared-state containers that wake the GUI on write.
//!
//! Wraps `ArcSwap<T>` / `AtomicU64` in a type whose `store` fires a
//! `RepaintSignal` atomically with the write. Writers cannot forget to wake
//! the GUI because the signal is baked into the container.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::{ArcSwap, Guard};

use crate::repaint::RepaintSignal;

pub struct Notified<T> {
    value: ArcSwap<T>,
    repaint: RepaintSignal,
}

impl<T> Notified<T> {
    pub fn new(initial: T, repaint: RepaintSignal) -> Self {
        Self {
            value: ArcSwap::from_pointee(initial),
            repaint,
        }
    }

    pub fn load(&self) -> Guard<Arc<T>> {
        self.value.load()
    }

    pub fn store(&self, new: T) {
        self.value.store(Arc::new(new));
        (self.repaint)();
    }
}

/// A value that round-trips losslessly through a `u64`.
pub trait AtomicBits: Copy {
    fn to_bits(self) -> u64;
    fn from_bits(bits: u64) -> Self;
}

/// An [`AtomicBits`] value whose bits occupy only the low 32 bits of the
/// `u64`.
pub trait AtomicBits32: AtomicBits {}

impl AtomicBits for bool {
    fn to_bits(self) -> u64 {
        self as u64
    }
    fn from_bits(bits: u64) -> Self {
        bits & 1 != 0
    }
}
impl AtomicBits32 for bool {}

impl AtomicBits for u32 {
    fn to_bits(self) -> u64 {
        self as u64
    }
    fn from_bits(bits: u64) -> Self {
        bits as u32
    }
}
impl AtomicBits32 for u32 {}

impl AtomicBits for f32 {
    fn to_bits(self) -> u64 {
        f32::to_bits(self) as u64
    }
    fn from_bits(bits: u64) -> Self {
        f32::from_bits(bits as u32)
    }
}
impl AtomicBits32 for f32 {}

/// Bit 32 flags `Some`, with the value in the low 32 bits; `None` is zero.
impl<T: AtomicBits32> AtomicBits for Option<T> {
    fn to_bits(self) -> u64 {
        const SOME: u64 = 1 << 32;
        self.map_or(0, |v| SOME | v.to_bits())
    }
    fn from_bits(bits: u64) -> Self {
        ((bits >> 32) & 1 != 0).then(|| T::from_bits(bits & u64::from(u32::MAX)))
    }
}

/// An atomic cell holding a `T`, whose stores fire the `RepaintSignal`, with
/// no heap allocation per update. Ordering is `Relaxed`: each value is a
/// self-contained payload.
pub struct NotifiedAtomic<T: AtomicBits> {
    bits: AtomicU64,
    repaint: RepaintSignal,
    _value: PhantomData<T>,
}

impl<T: AtomicBits> NotifiedAtomic<T> {
    pub fn new(initial: T, repaint: RepaintSignal) -> Self {
        Self {
            bits: AtomicU64::new(initial.to_bits()),
            repaint,
            _value: PhantomData,
        }
    }

    pub fn load(&self) -> T {
        T::from_bits(self.bits.load(Ordering::Relaxed))
    }

    pub fn store(&self, value: T) {
        self.bits.store(value.to_bits(), Ordering::Relaxed);
        (self.repaint)();
    }

    /// Store `value` if its bits differ from the current value's, firing the
    /// `RepaintSignal`; otherwise neither store nor fire.
    pub fn store_if_changed(&self, value: T) {
        if self.bits.load(Ordering::Relaxed) != value.to_bits() {
            self.store(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    fn counting_repaint() -> (RepaintSignal, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let count_for_signal = count.clone();
        let signal: RepaintSignal = Arc::new(move || {
            count_for_signal.fetch_add(1, Ordering::Relaxed);
        });
        (signal, count)
    }

    #[test]
    fn notified_store_fires_repaint_and_updates_value() {
        let (signal, count) = counting_repaint();
        let notified = Notified::new(1u32, signal);

        assert_eq!(**notified.load(), 1);
        assert_eq!(count.load(Ordering::Relaxed), 0);

        notified.store(2);
        assert_eq!(**notified.load(), 2);
        assert_eq!(count.load(Ordering::Relaxed), 1);

        // Storing the same value still fires — callers shouldn't assume the
        // container deduplicates, and "no change" is still a valid wake.
        notified.store(2);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn notified_store_fires_from_multiple_threads() {
        let (signal, count) = counting_repaint();
        let notified = Arc::new(Notified::new(0u32, signal));

        let handles: Vec<_> = (0..4)
            .map(|i| {
                let n = notified.clone();
                thread::spawn(move || n.store(i))
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(count.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn atomic_bits_round_trip() {
        fn round_trip<T: AtomicBits>(value: T) -> T {
            T::from_bits(value.to_bits())
        }
        for v in [false, true] {
            assert_eq!(round_trip(v), v);
        }
        for v in [0u32, 1, u32::MAX] {
            assert_eq!(round_trip(v), v);
        }
        for v in [0.0f32, -0.0, 3.5, f32::MAX, f32::NEG_INFINITY] {
            assert_eq!(round_trip(v).to_bits(), v.to_bits());
        }
        for v in [None, Some(false), Some(true)] {
            assert_eq!(round_trip(v), v);
        }
        for v in [None, Some(0u32), Some(u32::MAX)] {
            assert_eq!(round_trip(v), v);
        }
        for v in [None, Some(0.0f32), Some(-0.0), Some(-10.0)] {
            assert_eq!(round_trip(v).map(f32::to_bits), v.map(f32::to_bits));
        }
        assert!(round_trip(Some(f32::NAN)).is_some_and(f32::is_nan));
        assert_ne!(None::<bool>.to_bits(), Some(false).to_bits());
        assert_ne!(None::<f32>.to_bits(), Some(0.0f32).to_bits());
    }

    #[test]
    fn notified_atomic_fires_on_every_store_and_on_changed_bits_only() {
        let (signal, count) = counting_repaint();
        let cell = NotifiedAtomic::new(None::<f32>, signal);
        assert_eq!(cell.load(), None);

        cell.store(Some(1.0));
        assert_eq!(cell.load(), Some(1.0));
        cell.store(Some(1.0));
        assert_eq!(count.load(Ordering::Relaxed), 2, "store always fires");

        cell.store_if_changed(Some(1.0));
        assert_eq!(count.load(Ordering::Relaxed), 2, "unchanged bits");
        cell.store_if_changed(Some(0.0));
        assert_eq!(cell.load(), Some(0.0));
        assert_eq!(count.load(Ordering::Relaxed), 3, "changed bits");
        cell.store_if_changed(None);
        assert_eq!(cell.load(), None);
        assert_eq!(count.load(Ordering::Relaxed), 4, "Some(0.0) to None");
        cell.store_if_changed(Some(f32::NAN));
        cell.store_if_changed(Some(f32::NAN));
        assert!(cell.load().is_some_and(f32::is_nan));
        assert_eq!(count.load(Ordering::Relaxed), 5, "NaN stored twice");
    }

    #[test]
    fn noop_repaint_is_safe_to_call() {
        use crate::repaint::noop_repaint;
        let signal = noop_repaint();
        let notified = Notified::new(0u32, signal);
        notified.store(1);
        assert_eq!(**notified.load(), 1);
    }
}
