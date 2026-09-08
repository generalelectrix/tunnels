//! Nested rings of swept blades, on blade counts with no common factor.
//!
//! A straight blade is a spoke, and one ring of evenly spaced marks is what a
//! beam already makes with live knobs on every parameter. Two things take this
//! out of that reach: the blade sweeps, so it has handedness a segment has not,
//! and the layers carry coprime blade counts, so no single count describes the
//! figure. One layer alone falls back inside a beam's reach.

use crate::curve::ribbon;
use crate::geom::{CENTER, Figure, Point};
use std::f64::consts::TAU;

/// Samples along each blade.
const BLADE_STEPS: u32 = 24;

/// How far each layer is turned against the one inside it.
const LAYER_PHASE: f64 = 0.37;

/// One ring of blades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    /// Blades in this ring.
    pub blades: u32,
    /// Radius each blade starts at.
    pub inner: f64,
    /// Radius each blade ends at.
    pub outer: f64,
    /// How far a blade turns between those radii, in radians.
    pub sweep: f64,
    /// Width of the ribbon each blade is drawn as.
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PinwheelNest {
    pub layers: Vec<Layer>,
}

impl PinwheelNest {
    pub fn new(layers: Vec<Layer>) -> Self {
        Self { layers }
    }

    pub fn generate(&self) -> Figure {
        let contours = self
            .layers
            .iter()
            .enumerate()
            .flat_map(|(i, layer)| layer.contours(LAYER_PHASE * i as f64))
            .collect();
        Figure::even_odd(contours)
    }
}

impl Layer {
    pub const fn new(blades: u32, inner: f64, outer: f64, sweep: f64, width: f64) -> Self {
        Self {
            blades,
            inner,
            outer,
            sweep,
            width,
        }
    }

    fn contours(&self, phase: f64) -> Vec<crate::geom::Contour> {
        let span = self.outer - self.inner;
        if span == 0.0 {
            return Vec::new();
        }
        (0..self.blades)
            .flat_map(|j| {
                let start = phase + TAU * j as f64 / self.blades as f64;
                let arc: Vec<Point> = (0..=BLADE_STEPS)
                    .map(|i| {
                        let radius = self.inner + span * i as f64 / BLADE_STEPS as f64;
                        let angle = start + self.sweep * (radius - self.inner) / span;
                        Point::new(CENTER + radius * angle.cos(), CENTER + radius * angle.sin())
                    })
                    .collect();
                ribbon(&arc, self.width, false)
            })
            .collect()
    }
}
