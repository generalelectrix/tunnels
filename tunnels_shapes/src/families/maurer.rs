//! A rose walked in fixed angular steps rather than traced.
//!
//! The walk nearly misses itself, and the near-miss is the whole figure. An odd
//! petal count sends the walk over every point twice — the rose identity that
//! puts `(θ + π, −r)` exactly where `(θ, r)` already is — so it is walked half as
//! far, or it would cancel against itself.
//!
//! The walk closes after `360 / gcd(step, 360)` points, so the only lever on how
//! many chords it carries is a step sharing a factor with 360. At those steps
//! the few angles it lands on carry nearly equal radii and the figure collapses
//! into a chevron, which is why the step must be coprime with 360 and why this
//! family has no low-arity member.

use crate::curve::{FIT_RADIUS, fit, ribbon};
use crate::geom::{Figure, Point};
use std::f64::consts::PI;

/// Degrees to radians, as one multiplication.
const DEG_TO_RAD: f64 = PI / 180.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaurerRose {
    /// Petal count of the rose being walked.
    pub petals: u32,
    /// Angle advanced per step, in degrees.
    pub step_degrees: u32,
    /// Width of the ribbon the walk is drawn as.
    pub width: f64,
}

impl MaurerRose {
    pub const fn new(petals: u32, step_degrees: u32, width: f64) -> Self {
        Self {
            petals,
            step_degrees,
            width,
        }
    }

    pub fn generate(&self) -> Figure {
        let span = if self.petals.is_multiple_of(2) {
            360
        } else {
            180
        };
        let points: Vec<Point> = (0..=span)
            .map(|i| {
                let angle = (i * self.step_degrees) as f64 * DEG_TO_RAD;
                let radius = (self.petals as f64 * angle).sin();
                Point::new(radius * angle.cos(), radius * angle.sin())
            })
            .collect();
        Figure::even_odd(ribbon(&fit(&points, FIT_RADIUS), self.width, false))
    }
}
