//! Concentric polygon rings, each turned a little against the one outside it.
//!
//! The turn is what makes this more than nested polygons: corners drift around
//! the figure, so the eye follows a spiral that no single ring contains. Below
//! two rings a relative turn has nothing to register against.

use crate::curve::{ribbon, ring_points};
use crate::geom::Figure;
use std::f64::consts::FRAC_PI_2;

/// The radius of the outermost ring.
pub const OUTER_RADIUS: f64 = 470.0;

/// How far the stack reaches inwards from the outermost ring.
pub const DEPTH: f64 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwistRings {
    /// Corners on each ring.
    pub sides: u32,
    /// Rings in the stack.
    pub rings: u32,
    /// Angle each ring is turned against the one outside it, in radians.
    pub step: f64,
    /// Width of the ribbon each ring is drawn as.
    pub width: f64,
}

impl TwistRings {
    pub const fn new(sides: u32, rings: u32, step: f64, width: f64) -> Self {
        Self {
            sides,
            rings,
            step,
            width,
        }
    }

    pub fn generate(&self) -> Figure {
        if self.rings == 0 {
            return Figure::even_odd(Vec::new());
        }
        let pitch = DEPTH / self.rings as f64;
        let contours = (0..self.rings)
            .flat_map(|i| {
                let radius = OUTER_RADIUS - pitch * i as f64;
                let phase = -FRAC_PI_2 + self.step * i as f64;
                let points = ring_points(self.sides as usize, radius, phase);
                ribbon(&points, self.width, true)
            })
            .collect();
        Figure::even_odd(contours)
    }
}
