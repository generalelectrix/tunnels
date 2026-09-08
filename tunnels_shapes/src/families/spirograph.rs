//! A pen offset from a circle rolling inside or outside another.
//!
//! The three radii set the symmetry order and how deeply the loops cut back
//! through the middle, which is the whole of the interior structure. The lobe
//! count is `R / gcd(R, r)` — derived from the radii rather than set directly.

use crate::curve::{FIT_RADIUS, fit, ribbon, sample};
use crate::geom::Figure;
use std::f64::consts::TAU;

/// Samples taken across the closed curve.
pub const STEPS: usize = 760;

/// Which side of the fixed circle the rolling circle runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Roll {
    /// Inside: a hypotrochoid.
    Inside,
    /// Outside: an epitrochoid.
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spirograph {
    /// Radius of the fixed circle.
    pub fixed: u32,
    /// Radius of the rolling circle.
    pub rolling: u32,
    /// Distance of the pen from the rolling circle's centre.
    pub pen: u32,
    pub roll: Roll,
    /// Width of the ribbon the curve is drawn as.
    pub width: f64,
}

impl Spirograph {
    pub const fn new(fixed: u32, rolling: u32, pen: u32, roll: Roll, width: f64) -> Self {
        Self {
            fixed,
            rolling,
            pen,
            roll,
            width,
        }
    }

    /// The centreline of the curve, before it is given width.
    pub fn centreline(&self) -> Vec<crate::geom::Point> {
        if self.rolling == 0 {
            return Vec::new();
        }
        let (r, d) = (self.rolling as f64, self.pen as f64);
        let arm = match self.roll {
            Roll::Inside => self.fixed as f64 - r,
            Roll::Outside => self.fixed as f64 + r,
        };
        let k = arm / r;
        let period = TAU * r / gcd(self.fixed, self.rolling) as f64;
        let points = sample(
            |t| match self.roll {
                Roll::Inside => (
                    arm * t.cos() + d * (k * t).cos(),
                    arm * t.sin() - d * (k * t).sin(),
                ),
                Roll::Outside => (
                    arm * t.cos() - d * (k * t).cos(),
                    arm * t.sin() - d * (k * t).sin(),
                ),
            },
            0.0,
            period,
            STEPS,
        );
        fit(&points, FIT_RADIUS)
    }

    /// How many lobes the curve carries.
    pub fn lobes(&self) -> u32 {
        match gcd(self.fixed, self.rolling) {
            0 => 0,
            g => self.fixed / g,
        }
    }

    pub fn generate(&self) -> Figure {
        Figure::even_odd(ribbon(&self.centreline(), self.width, true))
    }
}

pub(crate) fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}
