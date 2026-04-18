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
