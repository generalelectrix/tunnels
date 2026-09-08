//! The chunky radial figures: rings, polygon stacks and petal mandalas.
//!
//! Each is holes rather than crossings — nested contours that an even-odd fill
//! resolves into alternating bands, with no two edges meeting anywhere.

use crate::curve::{Winding, circle, ring_points};
use crate::geom::{CENTER, Contour, Figure, Point};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Concentric circles, filling and cutting in turn.
#[derive(Debug, Clone, PartialEq)]
pub struct ConcentricRings {
    /// Radii from the outside in.
    pub radii: Vec<f64>,
}

impl ConcentricRings {
    pub fn new(radii: Vec<f64>) -> Self {
        Self { radii }
    }

    pub fn generate(&self) -> Figure {
        let centre = Point::new(CENTER, CENTER);
        let contours = self
            .radii
            .iter()
            .enumerate()
            .map(|(i, &radius)| {
                // Winding alternates so the rings cut holes under a non-zero
                // fill as well. Under even-odd the alternation makes no
                // difference, and the parity does the same work by itself.
                let winding = if i % 2 == 0 {
                    Winding::Positive
                } else {
                    Winding::Negative
                };
                circle(centre, radius, winding)
            })
            .collect();
        Figure::even_odd(contours)
    }
}

/// Concentric regular polygons: the scaffold a mandala is built on.
#[derive(Debug, Clone, PartialEq)]
pub struct PolygonRings {
    /// Corners on each ring.
    pub sides: u32,
    /// Radii from the outside in.
    pub radii: Vec<f64>,
}

impl PolygonRings {
    pub fn new(sides: u32, radii: Vec<f64>) -> Self {
        Self { sides, radii }
    }

    pub fn generate(&self) -> Figure {
        let contours = self
            .radii
            .iter()
            .enumerate()
            .map(|(i, &radius)| {
                let ring = Contour::new(ring_points(self.sides as usize, radius, -FRAC_PI_2));
                if i % 2 == 0 { ring } else { ring.reversed() }
            })
            .collect();
        Figure::even_odd(contours)
    }
}

/// Overlapping circles on a ring, meeting in lens-shaped petals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetalMandala {
    /// Petals around the ring.
    pub petals: u32,
    /// Radius the petal centres sit at.
    pub ring_radius: f64,
    /// Radius of each petal.
    pub petal_radius: f64,
}

impl PetalMandala {
    pub const fn new(petals: u32, ring_radius: f64, petal_radius: f64) -> Self {
        Self {
            petals,
            ring_radius,
            petal_radius,
        }
    }

    pub fn generate(&self) -> Figure {
        let contours = (0..self.petals)
            .map(|i| {
                let angle = TAU * i as f64 / self.petals as f64;
                let centre = Point::new(
                    CENTER + self.ring_radius * angle.cos(),
                    CENTER + self.ring_radius * angle.sin(),
                );
                circle(centre, self.petal_radius, Winding::Positive)
            })
            .collect();
        Figure::even_odd(contours)
    }
}
