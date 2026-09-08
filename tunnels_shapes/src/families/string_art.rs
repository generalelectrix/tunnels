//! Chords between neighbouring edges of a polygon.
//!
//! The curve nobody draws — a parabolic caustic standing in each corner as the
//! envelope of straight lines.

use crate::curve::{chord, on_ring};
use crate::geom::{Figure, Point};
use std::f64::consts::FRAC_PI_2;

/// The radius the polygon's corners sit at.
pub const RADIUS: f64 = 470.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StringArt {
    /// Corners of the polygon the chords span.
    pub sides: u32,
    /// Chords drawn into each corner.
    pub per_corner: u32,
    /// Width of each chord.
    pub width: f64,
}

impl StringArt {
    pub const fn new(sides: u32, per_corner: u32, width: f64) -> Self {
        Self {
            sides,
            per_corner,
            width,
        }
    }

    pub fn generate(&self) -> Figure {
        let sides = self.sides as usize;
        if sides == 0 || self.per_corner == 0 {
            return Figure::even_odd(Vec::new());
        }
        let corners: Vec<Point> = (0..sides)
            .map(|i| on_ring(i, sides, RADIUS, -FRAC_PI_2))
            .collect();
        let mut contours = Vec::new();
        for s in 0..sides {
            let (a0, a1) = (corners[s], corners[(s + 1) % sides]);
            let (b0, b1) = (corners[(s + 1) % sides], corners[(s + 2) % sides]);
            for i in 1..self.per_corner {
                let t = i as f64 / self.per_corner as f64;
                let along = Point::new(a0.x + (a1.x - a0.x) * t, a0.y + (a1.y - a0.y) * t);
                let back = Point::new(
                    b0.x + (b1.x - b0.x) * (1.0 - t),
                    b0.y + (b1.y - b0.y) * (1.0 - t),
                );
                contours.extend(chord(along, back, self.width));
            }
        }
        Figure::even_odd(contours)
    }
}
