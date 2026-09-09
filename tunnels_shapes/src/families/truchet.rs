//! Quarter arcs joining the edge midpoints of a square tiling.
//!
//! A full field with no centre at all, which is the one thing a figure built
//! around a ring cannot be. Orientation comes from a hash of the cell, so the
//! tiling is scattered and still the same every time it is built.

use crate::curve::ribbon;
use crate::geom::{EXTENT, Figure, Point};
use std::f64::consts::TAU;

/// Samples around the full circle each quarter arc is cut from.
const ARC_STEPS: u32 = 64;

/// How far outside its cell an arc point may stray before it is cut.
const CELL_SLACK: f64 = 1.0;

/// An arc cut down to this many points or fewer has no length worth drawing.
const DEGENERATE_ARC_POINTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Truchet {
    /// Tiles across the frame.
    pub tiles: u32,
    /// Width of the ribbon each arc is drawn as.
    pub width: f64,
}

impl Truchet {
    pub const fn new(tiles: u32, width: f64) -> Self {
        Self { tiles, width }
    }

    pub fn generate(&self) -> Figure {
        if self.tiles == 0 {
            return Figure::even_odd(Vec::new());
        }
        let pitch = EXTENT / self.tiles as f64;
        let mut contours = Vec::new();
        for gy in 0..self.tiles {
            for gx in 0..self.tiles {
                let (x0, y0) = (gx as f64 * pitch, gy as f64 * pitch);
                let corners = if flipped(gx, gy) {
                    [Point::new(x0, y0), Point::new(x0 + pitch, y0 + pitch)]
                } else {
                    [Point::new(x0 + pitch, y0), Point::new(x0, y0 + pitch)]
                };
                for corner in corners {
                    let arc: Vec<Point> = (0..=ARC_STEPS)
                        .map(|k| {
                            let angle = TAU * k as f64 / ARC_STEPS as f64;
                            Point::new(
                                corner.x + pitch / 2.0 * angle.cos(),
                                corner.y + pitch / 2.0 * angle.sin(),
                            )
                        })
                        .filter(|p| {
                            p.x >= x0 - CELL_SLACK
                                && p.x <= x0 + pitch + CELL_SLACK
                                && p.y >= y0 - CELL_SLACK
                                && p.y <= y0 + pitch + CELL_SLACK
                        })
                        .collect();
                    if arc.len() > DEGENERATE_ARC_POINTS {
                        contours.extend(ribbon(&arc, self.width, false));
                    }
                }
            }
        }
        Figure::even_odd(contours)
    }
}

/// Which diagonal a cell's two arcs stand on.
fn flipped(gx: u32, gy: u32) -> bool {
    let hash = (gx as u64 * 73856093) ^ (gy as u64 * 19349663) ^ 0x9E37_79B9;
    (hash >> 3) & 1 == 1
}
