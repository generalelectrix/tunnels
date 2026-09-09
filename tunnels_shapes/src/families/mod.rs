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
use serde::{Deserialize, Serialize};

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

/// The construction a figure is built by, without the numbers that place it.
///
/// A set of parameters names one of these rather than a [`ShapeFamily`]: a
/// family is a construction with a fixed choice of its second degree of
/// freedom, and that choice leaves no trace in what it builds. Three families
/// draw epicycloid spirographs, so a spirograph's radii say which construction
/// made it while leaving open which family reached it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Generator {
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

impl Generator {
    /// The name this construction is written under.
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

/// One position of the family control: a construction with its second degree of
/// freedom pinned.
///
/// Where a construction's second freedom reaches figures that are different
/// enough to be worth turning a knob to, each of those is a family of its own
/// and the arity control walks the count inside it. So several families here
/// share a [`Generator`] and differ only in what they pin it at.
///
/// Names carry the pinned value where it means the same thing at every arity —
/// a corner count, a reach — and an index where it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ShapeFamily {
    StarPolygon,
    Rose1,
    Rose2,
    Rose3,
    SpirographInside1,
    SpirographInside2,
    SpirographInside3,
    SpirographOutside1,
    SpirographOutside2,
    SpirographOutside3,
    Guilloche,
    Phyllotaxis,
    ModularChords1,
    ModularChords2,
    ModularChords3,
    StringArt4,
    StringArt5,
    StringArt6,
    TwistRings3,
    TwistRings4,
    MoireRings,
    MoireGrid,
    MoireWeave,
    StarLattice55,
    StarLattice70,
    StarLattice85,
    PolygonRings3,
    PolygonRings4,
    PolygonRings5,
    PolygonRings6,
    PolygonRings7,
    ConcentricRings,
    PetalMandala1,
    PetalMandala2,
    PetalMandala3,
    Slats,
    Grid,
}

impl ShapeFamily {
    /// The name this family's figures are written under.
    pub fn name(self) -> &'static str {
        match self {
            Self::StarPolygon => "star",
            Self::Rose1 => "rose_1",
            Self::Rose2 => "rose_2",
            Self::Rose3 => "rose_3",
            Self::SpirographInside1 => "spiro_hypo_1",
            Self::SpirographInside2 => "spiro_hypo_2",
            Self::SpirographInside3 => "spiro_hypo_3",
            Self::SpirographOutside1 => "spiro_epi_1",
            Self::SpirographOutside2 => "spiro_epi_2",
            Self::SpirographOutside3 => "spiro_epi_3",
            Self::Guilloche => "guilloche",
            Self::Phyllotaxis => "phyllo",
            Self::ModularChords1 => "modmult_1",
            Self::ModularChords2 => "modmult_2",
            Self::ModularChords3 => "modmult_3",
            Self::StringArt4 => "stringart_4",
            Self::StringArt5 => "stringart_5",
            Self::StringArt6 => "stringart_6",
            Self::TwistRings3 => "twistring_3",
            Self::TwistRings4 => "twistring_4",
            Self::MoireRings => "moire_rings",
            Self::MoireGrid => "moire_grid",
            Self::MoireWeave => "moire_weave",
            Self::StarLattice55 => "starlattice_55",
            Self::StarLattice70 => "starlattice_70",
            Self::StarLattice85 => "starlattice_85",
            Self::PolygonRings3 => "poly_rings_3",
            Self::PolygonRings4 => "poly_rings_4",
            Self::PolygonRings5 => "poly_rings_5",
            Self::PolygonRings6 => "poly_rings_6",
            Self::PolygonRings7 => "poly_rings_7",
            Self::ConcentricRings => "rings",
            Self::PetalMandala1 => "mandala_petal_1",
            Self::PetalMandala2 => "mandala_petal_2",
            Self::PetalMandala3 => "mandala_petal_3",
            Self::Slats => "slats",
            Self::Grid => "grid",
        }
    }

    /// The construction this family draws.
    pub fn generator(self) -> Generator {
        match self {
            Self::StarPolygon => Generator::StarPolygon,
            Self::Rose1 | Self::Rose2 | Self::Rose3 => Generator::Rose,
            Self::SpirographInside1 | Self::SpirographInside2 | Self::SpirographInside3 => {
                Generator::SpirographInside
            }
            Self::SpirographOutside1 | Self::SpirographOutside2 | Self::SpirographOutside3 => {
                Generator::SpirographOutside
            }
            Self::Guilloche => Generator::Guilloche,
            Self::Phyllotaxis => Generator::Phyllotaxis,
            Self::ModularChords1 | Self::ModularChords2 | Self::ModularChords3 => {
                Generator::ModularChords
            }
            Self::StringArt4 | Self::StringArt5 | Self::StringArt6 => Generator::StringArt,
            Self::TwistRings3 | Self::TwistRings4 => Generator::TwistRings,
            Self::MoireRings => Generator::MoireRings,
            Self::MoireGrid => Generator::MoireGrid,
            Self::MoireWeave => Generator::MoireWeave,
            Self::StarLattice55 | Self::StarLattice70 | Self::StarLattice85 => {
                Generator::StarLattice
            }
            Self::PolygonRings3
            | Self::PolygonRings4
            | Self::PolygonRings5
            | Self::PolygonRings6
            | Self::PolygonRings7 => Generator::PolygonRings,
            Self::ConcentricRings => Generator::ConcentricRings,
            Self::PetalMandala1 | Self::PetalMandala2 | Self::PetalMandala3 => {
                Generator::PetalMandala
            }
            Self::Slats => Generator::Slats,
            Self::Grid => Generator::Grid,
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
    /// The construction these parameters were built by.
    pub fn generator(&self) -> Generator {
        match self {
            Self::StarPolygon(_) => Generator::StarPolygon,
            Self::Rose(_) => Generator::Rose,
            Self::Spirograph(p) => match p.roll {
                Roll::Inside => Generator::SpirographInside,
                Roll::Outside => Generator::SpirographOutside,
            },
            Self::Guilloche(_) => Generator::Guilloche,
            Self::Lissajous(_) => Generator::Lissajous,
            Self::MaurerRose(_) => Generator::MaurerRose,
            Self::Harmonograph(_) => Generator::Harmonograph,
            Self::CycloidRosette(_) => Generator::CycloidRosette,
            Self::Phyllotaxis(_) => Generator::Phyllotaxis,
            Self::ModularChords(_) => Generator::ModularChords,
            Self::StringArt(_) => Generator::StringArt,
            Self::TwistRings(_) => Generator::TwistRings,
            Self::MoireRings(_) => Generator::MoireRings,
            Self::MoireBars(p) => match p.kind {
                MoireKind::Grid => Generator::MoireGrid,
                MoireKind::Weave => Generator::MoireWeave,
            },
            Self::PinwheelNest(_) => Generator::PinwheelNest,
            Self::StarLattice(_) => Generator::StarLattice,
            Self::Truchet(_) => Generator::Truchet,
            Self::ConcentricRings(_) => Generator::ConcentricRings,
            Self::PolygonRings(_) => Generator::PolygonRings,
            Self::PetalMandala(_) => Generator::PetalMandala,
            Self::Slats(_) => Generator::Slats,
            Self::Grid(_) => Generator::Grid,
            Self::Frames(_) => Generator::Frames,
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
