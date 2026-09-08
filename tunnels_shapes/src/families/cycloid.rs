//! A cycloid stacked with turned copies of itself.
//!
//! One cycloid is a single loop with nothing inside it. Copies turned by a
//! fraction of the curve's own symmetry give it an interior; a turn of a whole
//! multiple of that symmetry would land one copy on another and cancel the pair.

use crate::curve::{FIT_RADIUS, fit, ribbon, sample, turn};
use crate::geom::Figure;
use std::f64::consts::TAU;

/// Samples taken across one full turn of the base curve.
pub const STEPS: usize = 900;

/// Which cycloid the rosette is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycloidKind {
    Astroid,
    Deltoid,
    Cardioid,
    Nephroid,
}

impl CycloidKind {
    /// The curve's own rotational symmetry, which sets how far a copy may turn.
    pub fn order(self) -> u32 {
        match self {
            Self::Astroid => 4,
            Self::Deltoid => 3,
            Self::Cardioid => 1,
            Self::Nephroid => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Astroid => "astroid",
            Self::Deltoid => "deltoid",
            Self::Cardioid => "cardioid",
            Self::Nephroid => "nephroid",
        }
    }

    fn at(self, t: f64) -> (f64, f64) {
        match self {
            Self::Astroid => (t.cos().powf(3.0), t.sin().powf(3.0)),
            Self::Deltoid => (
                2.0 * t.cos() + (2.0 * t).cos(),
                2.0 * t.sin() - (2.0 * t).sin(),
            ),
            Self::Cardioid => {
                let scale = 1.0 - t.cos();
                (scale * t.cos(), scale * t.sin())
            }
            Self::Nephroid => (
                3.0 * t.cos() - (3.0 * t).cos(),
                3.0 * t.sin() - (3.0 * t).sin(),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycloidRosette {
    pub kind: CycloidKind,
    /// How many turned copies the figure carries.
    pub copies: u32,
    /// Width of the ribbon each copy is drawn as.
    pub width: f64,
}

impl CycloidRosette {
    pub const fn new(kind: CycloidKind, copies: u32, width: f64) -> Self {
        Self {
            kind,
            copies,
            width,
        }
    }

    pub fn generate(&self) -> Figure {
        let divisor = (self.kind.order() * self.copies) as f64;
        if divisor == 0.0 {
            return Figure::even_odd(Vec::new());
        }
        let base = fit(&sample(|t| self.kind.at(t), 0.0, TAU, STEPS), FIT_RADIUS);
        let contours = (0..self.copies)
            .flat_map(|i| ribbon(&turn(&base, TAU * i as f64 / divisor), self.width, true))
            .collect();
        Figure::even_odd(contours)
    }
}
