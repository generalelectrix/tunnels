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

pub struct NotifiedAtomicBool {
    value: AtomicBool,
    repaint: RepaintSignal,
}

impl NotifiedAtomicBool {
    pub fn new(initial: bool, repaint: RepaintSignal) -> Self {
        Self {
            value: AtomicBool::new(initial),
            repaint,
        }
    }

    pub fn load(&self, order: Ordering) -> bool {
        self.value.load(order)
    }

    pub fn store(&self, new: bool, order: Ordering) {
        self.value.store(new, order);
        (self.repaint)();
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
    fn notified_atomic_bool_store_fires_repaint() {
        let (signal, count) = counting_repaint();
        let nab = NotifiedAtomicBool::new(false, signal);

        assert!(!nab.load(Ordering::Relaxed));
        assert_eq!(count.load(Ordering::Relaxed), 0);

        nab.store(true, Ordering::Relaxed);
        assert!(nab.load(Ordering::Relaxed));
        assert_eq!(count.load(Ordering::Relaxed), 1);

        // Storing the same value still fires.
        nab.store(true, Ordering::Relaxed);
        assert_eq!(count.load(Ordering::Relaxed), 2);
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
