//! The star polygon {n/k}: every k'th vertex of a regular n-gon, joined in one
//! closed walk.
//!
//! A step of one is the n-gon itself and a step of two an open star whose points
//! a ring of marks already cuts. From a step of three the chords cross inside,
//! and under an even-odd fill those crossings carve an interior of k−1
//! concentric bands that no ring of marks can make.

use crate::curve::ring_points;
use crate::geom::{Contour, Figure};
use std::f64::consts::FRAC_PI_2;

/// The radius the vertices sit at.
pub const RADIUS: f64 = 460.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarPolygon {
    /// Vertices of the underlying regular polygon.
    pub points: u32,
    /// How many vertices each chord advances.
    ///
    /// A step sharing a factor with `points` closes the walk early and retraces
    /// it, which an even-odd fill cancels to nothing.
    pub step: u32,
}

impl StarPolygon {
    pub const fn new(points: u32, step: u32) -> Self {
        Self { points, step }
    }

    pub fn generate(&self) -> Figure {
        let n = self.points as usize;
        if n == 0 {
            return Figure::even_odd(Vec::new());
        }
        let vertices = ring_points(n, RADIUS, -FRAC_PI_2);
        let mut walk = Vec::with_capacity(n);
        let mut i = 0usize;
        for _ in 0..n {
            walk.push(vertices[i]);
            i = (i + self.step as usize) % n;
        }
        Figure::even_odd(vec![Contour::new(walk)])
    }
}
