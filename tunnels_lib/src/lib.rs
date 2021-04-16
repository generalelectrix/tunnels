//! Code shared between the tunnels console and client.

use derive_more::{Add, Display, Div, Mul, Sub};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Timestamp used for expressing moments in time, has units of microseconds.
/// Normally computed by the show controller as the number of microseconds since
/// the show launched.
/// Signed type to support possible situations where we need to subtract one
/// timestamp from another and end up with a negative result.
#[derive(
    Copy,
    Eq,
    PartialEq,
    Hash,
    Serialize,
    Deserialize,
    Debug,
    Display,
    Clone,
    Add,
    Sub,
    Mul,
    Div,
    Ord,
    PartialOrd,
)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn since(start: Instant) -> Self {
        Self::from_duration(start.elapsed())
    }

    pub fn from_duration(d: Duration) -> Self {
        Self(d.as_micros() as i64)
    }

    // Step mutably increments this timestamp by the provided step.
    pub fn step(&mut self, step: Duration) {
        *self = *self + Self::from_duration(step);
    }
}

impl num_traits::cast::ToPrimitive for Timestamp {
    fn to_i64(&self) -> Option<i64> {
        return Some(self.0);
    }

    fn to_u64(&self) -> Option<u64> {
        if self.0 >= 0 {
            return Some(self.0 as u64);
        }
        None
    }
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

/// A command to draw a single arc segment.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArcSegment {
    pub level: f64,
    pub thickness: f64,
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    pub x: f64,
    pub y: f64,
    pub rad_x: f64,
    pub rad_y: f64,
    pub start: f64,
    pub stop: f64,
    pub rot_angle: f64,
}

impl PartialEq for ArcSegment {
    fn eq(&self, o: &Self) -> bool {
        almost_eq(self.level, o.level)
            && almost_eq(self.thickness, o.thickness)
            && almost_eq(self.sat, o.sat)
            && almost_eq(self.val, o.val)
            && almost_eq(self.x, o.x)
            && almost_eq(self.y, o.y)
            && almost_eq(self.rad_x, o.rad_x)
            && almost_eq(self.rad_y, o.rad_y)
            && angle_almost_eq(self.hue, o.hue)
            && angle_almost_eq(self.start, o.start)
            && angle_almost_eq(self.stop, o.stop)
            && angle_almost_eq(self.rot_angle, o.rot_angle)
    }
}

impl Eq for ArcSegment {}

pub type LayerCollection = Vec<Arc<Vec<ArcSegment>>>;

/// A complete single-frame video snapshot.
/// This is the top-level structure sent in each serialized frame.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub frame_number: u64,
    pub time: Timestamp,
    pub layers: LayerCollection,
}

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
pub fn assert_almost_eq(a: f64, b: f64) {
    assert!(almost_eq(a, b), "{} != {}", a, b);
}
