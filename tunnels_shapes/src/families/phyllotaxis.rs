//! A dot per floret, placed on the golden angle.
//!
//! The only family here that is not a ribbon, because a disc already has area.
//! Below roughly twenty florets the spiral is a scatter of dots.

use crate::geom::{CENTER, Contour, Figure, Point};
use std::f64::consts::PI;

/// Vertices per floret. Twelve reads as a disc at projection scale.
const DOT_SIDES: u32 = 12;

/// The radius the outermost floret sits at.
const SPREAD: f64 = 470.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phyllotaxis {
    /// How many florets the figure carries.
    pub florets: u32,
    /// Radius of the outermost floret. Inner florets are drawn smaller.
    pub dot: f64,
}

impl Phyllotaxis {
    pub const fn new(florets: u32, dot: f64) -> Self {
        Self { florets, dot }
    }

    pub fn generate(&self) -> Figure {
        let golden = PI * (3.0 - 5.0_f64.sqrt());
        let n = self.florets as f64;
        let contours = (0..self.florets)
            .map(|i| {
                let fraction = (i as f64 / n).sqrt();
                let radius = SPREAD * fraction;
                let angle = i as f64 * golden;
                let centre =
                    Point::new(CENTER + radius * angle.cos(), CENTER + radius * angle.sin());
                let size = self.dot * (0.35 + 0.65 * fraction);
                (0..DOT_SIDES)
                    .map(|k| {
                        let t = k as f64 * PI / 6.0;
                        Point::new(centre.x + size * t.cos(), centre.y + size * t.sin())
                    })
                    .collect::<Contour>()
            })
            .collect();
        Figure::even_odd(contours)
    }
}
