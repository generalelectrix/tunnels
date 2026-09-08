//! Point `i` on a ring joined to point `k·i mod N`.
//!
//! A field of chords across the interior, with the cardioid and its relatives
//! appearing as the envelope of the crossings rather than being drawn. Nothing
//! lies on the ring except the endpoints.

use crate::curve::{chord, on_ring};
use crate::geom::Figure;
use std::collections::HashSet;

/// The radius the endpoints sit at.
pub const RADIUS: f64 = 470.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModularChords {
    /// Points on the ring.
    pub points: u32,
    /// The multiplier the chords follow.
    pub multiplier: u32,
    /// Width of each chord.
    pub width: u32,
}

impl ModularChords {
    pub const fn new(points: u32, multiplier: u32, width: u32) -> Self {
        Self {
            points,
            multiplier,
            width,
        }
    }

    /// Each chord is drawn once.
    ///
    /// Where the multiplier squared is one modulo the point count the map pairs
    /// `i` with `k·i` and back again, so every chord would come up twice and an
    /// even-odd fill would cancel the figure entirely.
    pub fn generate(&self) -> Figure {
        let n = self.points as usize;
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut contours = Vec::new();
        for i in 0..n {
            let j = (self.multiplier as usize * i) % n;
            let pair = (i.min(j), i.max(j));
            if j == i || !seen.insert(pair) {
                continue;
            }
            contours.extend(chord(
                on_ring(i, n, RADIUS, 0.0),
                on_ring(j, n, RADIUS, 0.0),
                self.width as f64,
            ));
        }
        Figure::even_odd(contours)
    }
}
