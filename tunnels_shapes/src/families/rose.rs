//! The rose `r = cos(nθ/d)`: a curve of petals through the origin.
//!
//! The curve closes after `d` half-turns when `n·d` is odd and after twice that
//! otherwise. Traced any further it lies exactly on itself, and an even-odd fill
//! cancels the figure to nothing — so the period is computed, not guessed at.

use crate::curve::{FIT_RADIUS, fit, ribbon, sample};
use crate::geom::Figure;
use std::f64::consts::PI;

/// Samples taken across one period.
pub const STEPS: usize = 900;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rose {
    /// Numerator of the angular frequency; sets the petal count.
    pub petals: u32,
    /// Denominator of the angular frequency; how many turns the petals take to
    /// close.
    ///
    /// The period below is the one for a frequency already in lowest terms, so a
    /// divisor sharing a factor with the petal count sends the curve round its
    /// own figure more than once — `Rose::new(4, 2, w)` traces the two-petal
    /// rose twice, and an even-odd fill cancels it to nothing. Every curated
    /// rose is coprime, so no preset stands on this; the arity control excludes
    /// it so nothing reached live can either.
    pub divisor: u32,
    /// Width of the ribbon the curve is drawn as.
    pub width: f64,
}

impl Rose {
    pub const fn new(petals: u32, divisor: u32, width: f64) -> Self {
        Self {
            petals,
            divisor,
            width,
        }
    }

    /// The angle the curve closes at.
    fn period(&self) -> f64 {
        let doubled = if (self.petals * self.divisor).is_multiple_of(2) {
            2.0
        } else {
            1.0
        };
        PI * self.divisor as f64 * doubled
    }

    pub fn generate(&self) -> Figure {
        if self.divisor == 0 {
            return Figure::even_odd(Vec::new());
        }
        let k = self.petals as f64 / self.divisor as f64;
        let points = sample(
            |t| {
                let radius = (k * t).cos();
                (radius * t.cos(), radius * t.sin())
            },
            0.0,
            self.period(),
            STEPS,
        );
        Figure::even_odd(ribbon(&fit(&points, FIT_RADIUS), self.width, true))
    }
}
