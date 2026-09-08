//! The khatam motif: an {8/3} star at every lattice point and again at every
//! corner, reaching far enough that neighbours cross.
//!
//! The stars run past the edge of the frame, which is the point — the figure is
//! a field cut out of a tiling rather than an object standing in the middle of
//! one.

use crate::curve::ribbon;
use crate::geom::{EXTENT, Figure, Point};
use std::f64::consts::{FRAC_PI_8, TAU};

/// Points on each star.
const POINTS: usize = 8;

/// How many points each chord advances.
const STEP: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarLattice {
    /// Lattice cells across the frame.
    pub tiles: u32,
    /// Width of the ribbon each star is drawn as.
    pub width: f64,
    /// How far a star reaches, as a fraction of the lattice pitch.
    ///
    /// Above half a pitch neighbouring stars cross, which is what interlaces the
    /// tiling.
    pub reach: f64,
}

impl StarLattice {
    pub const fn new(tiles: u32, width: f64, reach: f64) -> Self {
        Self {
            tiles,
            width,
            reach,
        }
    }

    pub fn generate(&self) -> Figure {
        if self.tiles == 0 {
            return Figure::even_odd(Vec::new());
        }
        let pitch = EXTENT / self.tiles as f64;
        let radius = self.reach * pitch;
        let mut contours = Vec::new();
        for gy in 0..=self.tiles {
            for gx in 0..=self.tiles {
                let centre = Point::new(gx as f64 * pitch, gy as f64 * pitch);
                for phase in [0.0, FRAC_PI_8] {
                    let ring: Vec<Point> = (0..POINTS)
                        .map(|j| {
                            let angle = phase + TAU * j as f64 / POINTS as f64;
                            Point::new(
                                centre.x + radius * angle.cos(),
                                centre.y + radius * angle.sin(),
                            )
                        })
                        .collect();
                    let walk: Vec<Point> = (0..POINTS).map(|j| ring[(j * STEP) % POINTS]).collect();
                    contours.extend(ribbon(&walk, self.width, true));
                }
            }
        }
        Figure::even_odd(contours)
    }
}
