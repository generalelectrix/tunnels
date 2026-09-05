//! Code shared between the tunnels console and client.

pub mod bootstrap;
pub mod color;
pub mod notified;
pub mod number;
pub mod prompt;
pub mod repaint;
pub mod smooth;
pub mod transient_indicator;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A helper wrapper around an atomically-reference-counted atomic boolean.
/// Used to control program flow across multiple threads.
#[derive(Debug, Clone)]
pub struct RunFlag(Arc<AtomicBool>);

impl Default for RunFlag {
    fn default() -> Self {
        RunFlag(Arc::new(AtomicBool::new(true)))
    }
}

impl RunFlag {
    /// Return true if the program should continue.
    pub fn should_run(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Command the program to stop.
    pub fn stop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

const ALMOST_EQ_TOLERANCE: f64 = 0.000_000_1;

/// Whether two values agree to within the tolerance a test cares about.
#[inline(always)]
pub fn almost_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < ALMOST_EQ_TOLERANCE
}

/// Panic unless two values agree to within the tolerance a test cares about.
pub fn assert_almost_eq(a: f64, b: f64) {
    assert!(almost_eq(a, b), "{a} != {b}");
}
