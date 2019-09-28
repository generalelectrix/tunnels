use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use timesync::Seconds;

const ALMOST_EQ_TOLERANCE: f64 = 0.000_000_1;

/// True modulus operator.
#[inline(always)]
pub fn modulo(a: f64, b: f64) -> f64 {
    ((a % b) + b) % b
}

/// Minimum included angle between two unit angles.
/// Might be negative.
#[inline(always)]
pub fn min_included_angle(a: f64, b: f64) -> f64 {
    ((((b - a) % 1.0) + 1.5) % 1.0) - 0.5
}

/// Return True if two f64 are within 10^-6 of each other.
/// This is OK because all of our floats are on the unit range, so even though
/// this comparison is absolute it should be good enough for art.
#[inline(always)]
pub fn almost_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < ALMOST_EQ_TOLERANCE
}

/// Return True if the min included angle betwee two unit angles is less than
/// 10^-6.
#[inline(always)]
pub fn angle_almost_eq(a: f64, b: f64) -> bool {
    min_included_angle(a, b).abs() < ALMOST_EQ_TOLERANCE
}

/// Panic if a and b are not almost equal.
#[inline(always)]
pub fn assert_almost_eq(a: f64, b: f64) {
    assert!(almost_eq(a, b), "{} != {}", a, b);
}

/// A helper wrapper around an atomically-reference-counted atomic boolean.
/// Used to control program flow across multiple threads.
#[derive(Debug, Clone)]
pub struct RunFlag(Arc<AtomicBool>);

impl RunFlag {
    /// Create a flag set to run.
    pub fn new() -> Self {
        RunFlag(Arc::new(AtomicBool::new(true)))
    }

    /// Return true if the program should continue.
    pub fn should_run(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Command the program to stop.
    pub fn stop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}
