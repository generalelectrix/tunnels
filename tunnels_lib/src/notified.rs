//! Shared-state containers that wake the GUI on write.
//!
//! Wraps `ArcSwap<T>` / `AtomicBool` in a type whose `store` fires a
//! `RepaintSignal` atomically with the write. Writers cannot forget to wake
//! the GUI because the signal is baked into the container.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// An atomic cell whose value loads and stores as a named `Value` type.
/// Ordering is `Relaxed` internally and intentionally not exposed: each value is
/// a self-contained payload, so no stronger ordering is load-bearing.
pub trait AtomicValue {
    type Value: Copy;
    fn new(value: Self::Value) -> Self;
    fn load(&self) -> Self::Value;
    fn store(&self, value: Self::Value);
}

impl AtomicValue for AtomicBool {
    type Value = bool;
    fn new(value: bool) -> Self {
        AtomicBool::new(value)
    }
    fn load(&self) -> bool {
        AtomicBool::load(self, Ordering::Relaxed)
    }
    fn store(&self, value: bool) {
        AtomicBool::store(self, value, Ordering::Relaxed)
    }
}

/// Atomic sibling of `Notified<T>`: a store atomically writes the value and fires
/// the `RepaintSignal`, with no heap allocation per update.
pub struct NotifiedAtomic<A: AtomicValue> {
    value: A,
    repaint: RepaintSignal,
}

impl<A: AtomicValue> NotifiedAtomic<A> {
    pub fn new(initial: A::Value, repaint: RepaintSignal) -> Self {
        Self {
            value: A::new(initial),
            repaint,
        }
    }
    pub fn load(&self) -> A::Value {
        self.value.load()
    }
    pub fn store(&self, value: A::Value) {
        self.value.store(value);
        (self.repaint)();
    }
}

/// A `NotifiedAtomic` over a `bool`.
pub type NotifiedAtomicBool = NotifiedAtomic<AtomicBool>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize};
    use std::thread;

    // A non-bool `AtomicValue` impl proves the generic works over more than
    // `AtomicBool`. It lives in the test module rather than the library because
    // the real `AtomicU64` impl will be provided by a different crate.
    impl AtomicValue for AtomicU64 {
        type Value = u64;
        fn new(value: u64) -> Self {
            AtomicU64::new(value)
        }
        fn load(&self) -> u64 {
            AtomicU64::load(self, Ordering::Relaxed)
        }
        fn store(&self, value: u64) {
            AtomicU64::store(self, value, Ordering::Relaxed)
        }
    }

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
    fn notified_atomic_bool_store_fires_repaint() {
        let (signal, count) = counting_repaint();
        let nab = NotifiedAtomicBool::new(false, signal);

        assert!(!nab.load());
        assert_eq!(count.load(Ordering::Relaxed), 0);

        nab.store(true);
        assert!(nab.load());
        assert_eq!(count.load(Ordering::Relaxed), 1);

        // Storing the same value still fires.
        nab.store(true);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn notified_atomic_generic_over_non_bool() {
        let (signal, count) = counting_repaint();
        let cell: NotifiedAtomic<AtomicU64> = NotifiedAtomic::new(7, signal);

        assert_eq!(cell.load(), 7);
        assert_eq!(count.load(Ordering::Relaxed), 0);

        cell.store(42);
        assert_eq!(cell.load(), 42);
        assert_eq!(count.load(Ordering::Relaxed), 1);
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
