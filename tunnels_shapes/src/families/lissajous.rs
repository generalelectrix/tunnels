//! `x = sin(at + δ)`, `y = sin(bt)`: two perpendicular oscillations.
//!
//! A ratio of one to one is an ellipse, which a ring of marks already makes. A
//! quarter-turn phase is avoided where `a` is even and `b` odd: there the map
//! `t → π − t` returns the same point, so the figure is traced twice and cancels
//! against itself.

use crate::curve::{FIT_RADIUS, fit, ribbon, sample};
use crate::geom::Figure;
use std::f64::consts::TAU;

/// Samples taken across one full period.
pub const STEPS: usize = 1600;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lissajous {
    /// Frequency of the horizontal oscillation.
    pub a: u32,
    /// Frequency of the vertical oscillation.
    pub b: u32,
    /// Phase lead of the horizontal oscillation, in radians.
    pub delta: f64,
    /// Width of the ribbon the curve is drawn as.
    pub width: f64,
}

impl Lissajous {
    pub const fn new(a: u32, b: u32, delta: f64, width: f64) -> Self {
        Self { a, b, delta, width }
    }

    pub fn generate(&self) -> Figure {
        let (a, b) = (self.a as f64, self.b as f64);
        let points = sample(
            |t| ((a * t + self.delta).sin(), (b * t).sin()),
            0.0,
            TAU,
            STEPS,
        );
        Figure::even_odd(ribbon(&fit(&points, FIT_RADIUS), self.width, true))
    }
}
