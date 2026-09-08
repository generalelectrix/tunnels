//! One module per family of figures.
//!
//! A family is a construction, not a shape: it fixes how the contours are built
//! and leaves the numbers that place one figure inside it to [`ShapeParams`].

pub mod bars;
pub mod cycloid;
pub mod guilloche;
pub mod harmonograph;
pub mod lissajous;
pub mod maurer;
pub mod mod_mult;
pub mod moire;
pub mod phyllotaxis;
pub mod pinwheel;
pub mod radial;
pub mod rose;
pub mod spirograph;
pub mod star_lattice;
pub mod star_polygon;
pub mod string_art;
pub mod truchet;
pub mod twist_rings;

use crate::geom::Figure;

pub use bars::{Frames, Grid, Slats};
pub use cycloid::{CycloidKind, CycloidRosette};
pub use guilloche::Guilloche;
pub use harmonograph::Harmonograph;
pub use lissajous::Lissajous;
pub use maurer::MaurerRose;
pub use mod_mult::ModularChords;
pub use moire::{MoireBars, MoireKind, MoireRings};
pub use phyllotaxis::Phyllotaxis;
pub use pinwheel::PinwheelNest;
pub use radial::{ConcentricRings, PetalMandala, PolygonRings};
pub use rose::Rose;
pub use spirograph::{Roll, Spirograph};
pub use star_lattice::StarLattice;
pub use star_polygon::StarPolygon;
pub use string_art::StringArt;
pub use truchet::Truchet;
pub use twist_rings::TwistRings;

/// The family a figure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShapeFamily {
    StarPolygon,
    Rose,
    SpirographInside,
    SpirographOutside,
    Guilloche,
    Lissajous,
    MaurerRose,
    Harmonograph,
    CycloidRosette,
    Phyllotaxis,
    ModularChords,
    StringArt,
    TwistRings,
    MoireRings,
    MoireGrid,
    MoireWeave,
    PinwheelNest,
    StarLattice,
    Truchet,
    ConcentricRings,
    PolygonRings,
    PetalMandala,
    Slats,
    Grid,
    Frames,
}

impl ShapeFamily {
    /// The name this family's figures are written under.
    pub fn name(self) -> &'static str {
        match self {
            Self::StarPolygon => "star",
            Self::Rose => "rose",
            Self::SpirographInside => "spiro_hypo",
            Self::SpirographOutside => "spiro_epi",
            Self::Guilloche => "guilloche",
            Self::Lissajous => "liss",
            Self::MaurerRose => "maurer",
            Self::Harmonograph => "harmo",
            Self::CycloidRosette => "cyc",
            Self::Phyllotaxis => "phyllo",
            Self::ModularChords => "modmult",
            Self::StringArt => "stringart",
            Self::TwistRings => "twistring",
            Self::MoireRings => "moire_rings",
            Self::MoireGrid => "moire_grid",
            Self::MoireWeave => "moire_weave",
            Self::PinwheelNest => "pinwheel_nest",
            Self::StarLattice => "starlattice",
            Self::Truchet => "truchet",
            Self::ConcentricRings => "rings",
            Self::PolygonRings => "poly_rings",
            Self::PetalMandala => "mandala_petal",
            Self::Slats => "slats",
            Self::Grid => "grid",
            Self::Frames => "frames",
        }
    }
}

/// A figure's family together with the parameters that place it inside that family.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeParams {
    StarPolygon(StarPolygon),
    Rose(Rose),
    Spirograph(Spirograph),
    Guilloche(Guilloche),
    Lissajous(Lissajous),
    MaurerRose(MaurerRose),
    Harmonograph(Harmonograph),
    CycloidRosette(CycloidRosette),
    Phyllotaxis(Phyllotaxis),
    ModularChords(ModularChords),
    StringArt(StringArt),
    TwistRings(TwistRings),
    MoireRings(MoireRings),
    MoireBars(MoireBars),
    PinwheelNest(PinwheelNest),
    StarLattice(StarLattice),
    Truchet(Truchet),
    ConcentricRings(ConcentricRings),
    PolygonRings(PolygonRings),
    PetalMandala(PetalMandala),
    Slats(Slats),
    Grid(Grid),
    Frames(Frames),
}

impl ShapeParams {
    /// The family these parameters belong to.
    pub fn family(&self) -> ShapeFamily {
        match self {
            Self::StarPolygon(_) => ShapeFamily::StarPolygon,
            Self::Rose(_) => ShapeFamily::Rose,
            Self::Spirograph(p) => match p.roll {
                Roll::Inside => ShapeFamily::SpirographInside,
                Roll::Outside => ShapeFamily::SpirographOutside,
            },
            Self::Guilloche(_) => ShapeFamily::Guilloche,
            Self::Lissajous(_) => ShapeFamily::Lissajous,
            Self::MaurerRose(_) => ShapeFamily::MaurerRose,
            Self::Harmonograph(_) => ShapeFamily::Harmonograph,
            Self::CycloidRosette(_) => ShapeFamily::CycloidRosette,
            Self::Phyllotaxis(_) => ShapeFamily::Phyllotaxis,
            Self::ModularChords(_) => ShapeFamily::ModularChords,
            Self::StringArt(_) => ShapeFamily::StringArt,
            Self::TwistRings(_) => ShapeFamily::TwistRings,
            Self::MoireRings(_) => ShapeFamily::MoireRings,
            Self::MoireBars(p) => match p.kind {
                MoireKind::Grid => ShapeFamily::MoireGrid,
                MoireKind::Weave => ShapeFamily::MoireWeave,
            },
            Self::PinwheelNest(_) => ShapeFamily::PinwheelNest,
            Self::StarLattice(_) => ShapeFamily::StarLattice,
            Self::Truchet(_) => ShapeFamily::Truchet,
            Self::ConcentricRings(_) => ShapeFamily::ConcentricRings,
            Self::PolygonRings(_) => ShapeFamily::PolygonRings,
            Self::PetalMandala(_) => ShapeFamily::PetalMandala,
            Self::Slats(_) => ShapeFamily::Slats,
            Self::Grid(_) => ShapeFamily::Grid,
            Self::Frames(_) => ShapeFamily::Frames,
        }
    }

    /// Build the figure these parameters describe.
    pub fn generate(&self) -> Figure {
        match self {
            Self::StarPolygon(p) => p.generate(),
            Self::Rose(p) => p.generate(),
            Self::Spirograph(p) => p.generate(),
            Self::Guilloche(p) => p.generate(),
            Self::Lissajous(p) => p.generate(),
            Self::MaurerRose(p) => p.generate(),
            Self::Harmonograph(p) => p.generate(),
            Self::CycloidRosette(p) => p.generate(),
            Self::Phyllotaxis(p) => p.generate(),
            Self::ModularChords(p) => p.generate(),
            Self::StringArt(p) => p.generate(),
            Self::TwistRings(p) => p.generate(),
            Self::MoireRings(p) => p.generate(),
            Self::MoireBars(p) => p.generate(),
            Self::PinwheelNest(p) => p.generate(),
            Self::StarLattice(p) => p.generate(),
            Self::Truchet(p) => p.generate(),
            Self::ConcentricRings(p) => p.generate(),
            Self::PolygonRings(p) => p.generate(),
            Self::PetalMandala(p) => p.generate(),
            Self::Slats(p) => p.generate(),
            Self::Grid(p) => p.generate(),
            Self::Frames(p) => p.generate(),
        }
    }
}
