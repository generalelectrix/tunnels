//! Concentric polygon rings, each turned a little against the one outside it.
//!
//! The turn is what makes this more than nested polygons: corners drift around
//! the figure, so the eye follows a spiral that no single ring contains. Below
//! two rings a relative turn has nothing to register against.

use crate::curve::{ribbon, ring_points};
use crate::geom::{CENTER, Contour, Figure, Point};
use std::f64::consts::FRAC_PI_2;

/// A corner at the top of the frame, before the stack's own turn is added.
const CORNER_PHASE: f64 = -FRAC_PI_2;

/// The same contours, moved so that what they cover is centred on the frame.
///
/// A ring is placed by the circle its corners sit on, and that circle's centre
/// is the middle of what the ring covers only when a corner faces each of two
/// opposite edges. With an odd number of corners one edge is faced by a corner
/// and the other by a side, which reaches only `cos(pi / sides)` as far. Each
/// ring in a stack is turned against the one outside it, so the amount is not
/// the same for all of them and the stack as a whole is what gets centred.
fn centred(contours: Vec<Contour>) -> Vec<Contour> {
    let mut low = Point::new(f64::INFINITY, f64::INFINITY);
    let mut high = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for point in contours.iter().flat_map(|c| c.points()) {
        low = Point::new(low.x.min(point.x), low.y.min(point.y));
        high = Point::new(high.x.max(point.x), high.y.max(point.y));
    }
    if low.x > high.x {
        return contours;
    }
    let (dx, dy) = (
        CENTER - (low.x + high.x) / 2.0,
        CENTER - (low.y + high.y) / 2.0,
    );
    contours
        .into_iter()
        .map(|contour| {
            contour
                .points()
                .iter()
                .map(|p| Point::new(p.x + dx, p.y + dy))
                .collect()
        })
        .collect()
}

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
        let contours: Vec<Contour> = (0..self.rings)
            .flat_map(|i| {
                let radius = OUTER_RADIUS - pitch * i as f64;
                let phase = CORNER_PHASE + self.step * i as f64;
                let points = ring_points(self.sides as usize, radius, phase);
                ribbon(&points, self.width, true)
            })
            .collect();
        Figure::even_odd(centred(contours))
    }
}
