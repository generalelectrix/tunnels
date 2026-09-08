//! One spirograph laid over copies of itself.
//!
//! Each copy is turned by a fraction of the base curve's own symmetry, so the
//! copies interfere instead of coinciding. A turn of a whole multiple of that
//! symmetry would put a copy exactly on the original, which an even-odd fill
//! cancels to nothing.

use crate::curve::{ribbon, turn};
use crate::families::spirograph::Spirograph;
use crate::geom::Figure;
use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guilloche {
    /// The curve the copies are made from.
    pub base: Spirograph,
    /// How many copies the figure carries.
    pub copies: u32,
}

impl Guilloche {
    pub const fn new(base: Spirograph, copies: u32) -> Self {
        Self { base, copies }
    }

    pub fn generate(&self) -> Figure {
        let points = self.base.centreline();
        let order = self.base.lobes();
        let divisor = (order * self.copies) as f64;
        if divisor == 0.0 {
            return Figure::even_odd(Vec::new());
        }
        let contours = (0..self.copies)
            .flat_map(|i| {
                let turned = turn(&points, TAU * i as f64 / divisor);
                ribbon(&turned, self.base.width, true)
            })
            .collect();
        Figure::even_odd(contours)
    }
}
