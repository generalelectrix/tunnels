//! One module per family of figures.

pub mod star_polygon;

use crate::geom::Figure;

pub use star_polygon::StarPolygon;

/// The family a figure belongs to.
///
/// A family is a construction, not a shape: it fixes how the contours are built
/// and leaves the numbers that place one figure inside it to [`ShapeParams`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeFamily {
    StarPolygon,
}

impl ShapeFamily {
    /// The name this family is written under.
    pub fn name(self) -> &'static str {
        match self {
            Self::StarPolygon => "star",
        }
    }
}

/// A figure's family together with the parameters that place it inside that family.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeParams {
    StarPolygon(StarPolygon),
}

impl ShapeParams {
    /// The family these parameters belong to.
    pub fn family(&self) -> ShapeFamily {
        match self {
            Self::StarPolygon(_) => ShapeFamily::StarPolygon,
        }
    }

    /// Build the figure these parameters describe.
    pub fn generate(&self) -> Figure {
        match self {
            Self::StarPolygon(p) => p.generate(),
        }
    }
}
