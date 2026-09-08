//! Reinterpreting two abstract controls in each family's own domain.
//!
//! A figure is reached with two numbers, not with a parameter struct per family.
//! **Arity** is the count the family is built around — points on a star, lobes
//! on a spirograph, florets in a phyllotaxis — and **secondary** is a position
//! between zero and one that each family resolves into whatever its second
//! degree of freedom is.
//!
//! Every other number a family carries follows from those two. The hand-tuned
//! third columns the figures were originally written with are all one law: the
//! marks must not merge, so a ribbon's width, a dot's radius or a grid's angle
//! scales as the inverse of arity. Where that law was fitted against the
//! curated figures the fit is named at the point it is used; where the curated
//! set was too small to carry a law, the observed band is named instead and the
//! value taken from the middle of it, so the choice can be checked rather than
//! trusted.

use crate::families::*;
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_3, TAU};
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;

/// The count a family is built around, resolved in that family's own domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Arity(u32);

impl Arity {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    /// Where this arity sits in a range, as a fraction from zero to one.
    fn across(self, range: &RangeInclusive<u32>) -> f64 {
        let span = range.end().saturating_sub(*range.start());
        match span {
            0 => 0.0,
            span => {
                (self.0.clamp(*range.start(), *range.end()) - range.start()) as f64 / span as f64
            }
        }
    }
}

/// A family's second degree of freedom, as a position between zero and one.
///
/// What it selects differs by family — a step, a divisor, an orientation, a
/// curve — so it arrives without units and is resolved on the way in.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(from = "f64")]
pub struct Secondary(f64);

impl Secondary {
    pub fn new(value: f64) -> Self {
        Self(if value.is_finite() {
            // Adding zero is what folds a negative zero onto the positive one.
            // Both compare equal, and a position that names the same figure
            // has to hash to the same place as well as compare equal to it.
            value.clamp(0.0, 1.0) + 0.0
        } else {
            0.0
        })
    }

    pub const fn get(self) -> f64 {
        self.0
    }

    /// The item this position selects from a list.
    fn pick<T: Copy>(self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            return None;
        }
        let last = items.len() - 1;
        items
            .get(((self.0 * items.len() as f64) as usize).min(last))
            .copied()
    }

    /// The whole number this position selects from an inclusive range.
    fn pick_in(self, range: RangeInclusive<u32>) -> u32 {
        let values: Vec<u32> = range.collect();
        self.pick(&values).unwrap_or(0)
    }

    /// This position mapped onto an interval.
    fn between(self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.0
    }
}

/// A position is a number, and two of them that came out of the same
/// arithmetic agree bit for bit; two that merely landed close are different
/// figures and stay so.
impl Eq for Secondary {}

impl Hash for Secondary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl From<f64> for Secondary {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

/// A value falling off with arity, anchored at the two ends of a family's range.
///
/// Bold at the low end and fine at the high end, which is the same law as every
/// hand-tuned width in the library: the marks must not merge.
fn taper(arity: Arity, range: &RangeInclusive<u32>, bold: f64, fine: f64) -> f64 {
    bold + (fine - bold) * arity.across(range)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl ShapeFamily {
    /// Every family, in the order the enum declares them.
    ///
    /// A new family has to be added here as well as to the enum. The compiler
    /// catches a family missing from `name`, `resolve` and `ShapeParams::family`,
    /// which are exhaustive matches, but not one missing from this list;
    /// `every_family_is_reachable` is what catches that.
    pub const ALL: [ShapeFamily; 25] = [
        Self::StarPolygon,
        Self::Rose,
        Self::SpirographInside,
        Self::SpirographOutside,
        Self::Guilloche,
        Self::Lissajous,
        Self::MaurerRose,
        Self::Harmonograph,
        Self::CycloidRosette,
        Self::Phyllotaxis,
        Self::ModularChords,
        Self::StringArt,
        Self::TwistRings,
        Self::MoireRings,
        Self::MoireGrid,
        Self::MoireWeave,
        Self::PinwheelNest,
        Self::StarLattice,
        Self::Truchet,
        Self::ConcentricRings,
        Self::PolygonRings,
        Self::PetalMandala,
        Self::Slats,
        Self::Grid,
        Self::Frames,
    ];

    /// The arities this family is a figure over.
    ///
    /// Below the low end each family stops being itself and becomes a ring of
    /// marks a beam already makes; the bounds are where that bites.
    pub fn arity_range(self) -> RangeInclusive<u32> {
        match self {
            // A step of two is an open star; from three the chords cross, and
            // seven is the fewest points a step of three has room in.
            Self::StarPolygon => 7..=19,
            // One petal is a circle through the origin.
            Self::Rose => 2..=13,
            // Two lobes is an ellipse.
            Self::SpirographInside | Self::SpirographOutside => 3..=21,
            // One copy is a plain spirograph with nothing to interfere with.
            Self::Guilloche => 2..=8,
            // One to one is an ellipse.
            Self::Lissajous => 2..=13,
            Self::MaurerRose => 2..=15,
            // No arity axis; the control selects among the curated figures.
            Self::Harmonograph => 1..=13,
            // One copy is a bare loop with nothing inside it.
            Self::CycloidRosette => 2..=12,
            // Below about twenty florets the spiral is a scatter of dots.
            Self::Phyllotaxis => 24..=400,
            // Below about twenty points the chords outline a polygon.
            Self::ModularChords => 24..=220,
            Self::StringArt => 5..=34,
            // Two rings, below which a relative turn registers against nothing.
            Self::TwistRings => 3..=20,
            // Too few of either and there is no second pattern to beat against.
            Self::MoireRings => 3..=26,
            // The upper end muds to grey at forty feet, which is a viewing
            // distance rather than a geometric bound.
            Self::MoireGrid => 8..=48,
            Self::MoireWeave => 6..=40,
            // One layer is a ring of blades a beam makes with live knobs, and
            // two blades on the outside leaves no coprime count under it.
            Self::PinwheelNest => 3..=17,
            Self::StarLattice => 1..=5,
            Self::Truchet => 2..=16,
            Self::ConcentricRings
            | Self::PolygonRings
            | Self::PetalMandala
            | Self::Slats
            | Self::Grid
            | Self::Frames => 4..=16,
        }
    }

    /// The arities a control offers in this family.
    ///
    /// A knob names one of these rather than any value in [`arity_range`],
    /// because every figure a knob can reach is built and held: the count is
    /// what the library costs. Sixteen of the families are short enough to be
    /// offered whole; the nine that are not are offered the counts the curated
    /// figures were drawn at, which is a subset and never a ceiling. Clamping
    /// instead would put `phyllo` and `modmult` below the counts their
    /// construction needs — a phyllotaxis under about twenty florets is a
    /// scatter of dots rather than a coarse spiral.
    ///
    /// Where curation left a gap wider than its neighbours, the gap is filled
    /// so the step between one position and the next stays a roughly constant
    /// proportion. That is the spacing the curated counts already have, and
    /// the reason for it is that arity is perceived as a ratio: eight bars to
    /// ten is a visible change where forty to forty-two is not.
    pub fn arities(self) -> &'static [Arity] {
        macro_rules! arities {
            ($($n:literal),* $(,)?) => {{
                const ARITIES: &[Arity] = &[$(Arity::new($n)),*];
                ARITIES
            }};
        }
        match self {
            Self::StarPolygon => arities![7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19],
            Self::Rose => arities![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            // Every lobe count to fifteen, then every other one: the curated
            // spirographs stop at seventeen inside and outside, and two lobes
            // apart is where a count still reads as a different figure.
            Self::SpirographInside | Self::SpirographOutside => {
                arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 19, 21]
            }
            Self::Guilloche => arities![2, 3, 4, 5, 6, 7, 8],
            Self::Lissajous => arities![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            Self::MaurerRose => arities![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
            Self::Harmonograph => arities![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            Self::CycloidRosette => arities![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            // The sixteen curated floret counts exactly. The dot radius is
            // fitted against this count, and these are the counts it was
            // fitted on.
            Self::Phyllotaxis => {
                arities![
                    24, 30, 36, 45, 55, 60, 75, 90, 120, 150, 180, 200, 240, 260, 300, 400
                ]
            }
            // The curated point counts, less four that sat beside a neighbour
            // close enough to draw the same figure.
            Self::ModularChords => {
                arities![
                    24, 30, 36, 45, 48, 56, 60, 90, 96, 120, 128, 144, 150, 180, 200, 220
                ]
            }
            // The sixteen curated corner counts exactly.
            Self::StringArt => arities![5, 6, 7, 8, 9, 10, 12, 13, 14, 16, 18, 20, 22, 26, 30, 34],
            Self::TwistRings => arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20],
            Self::MoireRings => arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 16, 19, 22, 24, 26],
            Self::MoireGrid => {
                arities![8, 9, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 34, 40, 48]
            }
            Self::MoireWeave => {
                arities![6, 7, 8, 9, 10, 11, 12, 14, 16, 18, 20, 23, 26, 29, 32, 40]
            }
            Self::PinwheelNest => arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
            Self::StarLattice => arities![1, 2, 3, 4, 5],
            Self::Truchet => arities![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            Self::ConcentricRings
            | Self::PolygonRings
            | Self::PetalMandala
            | Self::Slats
            | Self::Grid
            | Self::Frames => arities![4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        }
    }

    /// The second degree of freedom every figure of this family is drawn at.
    ///
    /// One position rather than a range, because the two controls a figure
    /// mode has are spent on the family and the count within it. Which
    /// position is a matter of taste, and the curated figures are the evidence
    /// there is about taste: each of these is the position whose resolution
    /// agrees with the most curated figures of its family, and where two agree
    /// equally the tie goes to the one curation kept at the top of the arity
    /// range. Where the position carries units — a pen depth, a reach, an
    /// overlap — it is the median of the curated values instead.
    ///
    /// Nine families resolve nothing from this and take zero: `harmo`,
    /// `phyllo`, `moire_grid`, `moire_weave`, `pinwheel_nest`, `truchet`,
    /// `rings`, `grid` and `frames`.
    pub fn secondary(self) -> Secondary {
        Secondary::new(match self {
            // A step near two fifths of the point count. Curation kept only
            // the high steps above fifteen points and none of the low ones.
            Self::StarPolygon => 0.79,
            Self::Rose => 0.646,
            // A pen depth of 1.74, against a curated median of 1.75. Depth one
            // is the cusped hypocycloid, where a ribbon leaves the tips as
            // detached blobs, and this is well clear of it.
            Self::SpirographInside => 0.37,
            // A pen depth of 1.56; the curated epicycloids run shallower than
            // the hypocycloids, at a median of 1.50.
            Self::SpirographOutside => 0.28,
            // The simplest base. A guilloche's marks are the base's lobes
            // times its copies, and the copies are what the other knob turns.
            Self::Guilloche => 0.045,
            // Every curated Lissajous has its second frequency below its
            // first, and at two the coprime frequencies are 1, 3, 5, 7 and 9 —
            // so any position above a fifth leaves that region at the bottom
            // of the range.
            Self::Lissajous => 0.183,
            // A step of 53, the only one two curated roses share.
            Self::MaurerRose => 0.35,
            // The cardioid: one cusp where the nephroid has two, and the only
            // kind curation carried to twelve copies.
            Self::CycloidRosette => 0.625,
            // A multiplier near a twelfth of the point count, the median of
            // the curated ratios.
            Self::ModularChords => 0.086,
            // Five corners, which holds the marks a figure carries — corners
            // times the count per corner — closest to the curated band across
            // the whole range.
            Self::StringArt => 0.214,
            // Five sides, the only count curation carried to twenty rings.
            Self::TwistRings => 0.178,
            // One ring of offset: the longest beat there is, and the only
            // offset coprime with every ring count, so it does not jump as the
            // count crosses a factor.
            Self::MoireRings => 0.167,
            // A reach of 0.70, which five curated lattices carry.
            Self::StarLattice => 0.5,
            // Six sides. A stack of circles is already its own family, and an
            // octagon is nearly one.
            Self::PolygonRings => 0.35,
            // Petals overlapping by one spacing, where each petal's rim passes
            // through its neighbours' centres.
            Self::PetalMandala => 0.75,
            Self::Slats => 0.25,
            Self::Harmonograph
            | Self::Phyllotaxis
            | Self::MoireGrid
            | Self::MoireWeave
            | Self::PinwheelNest
            | Self::Truchet
            | Self::ConcentricRings
            | Self::Grid
            | Self::Frames => 0.0,
        })
    }

    /// The parameters this family reaches from an arity and a secondary.
    pub fn resolve(self, arity: Arity, secondary: Secondary) -> ShapeParams {
        let range = self.arity_range();
        let n = arity.get().clamp(*range.start(), *range.end());
        match self {
            Self::StarPolygon => {
                ShapeParams::StarPolygon(StarPolygon::new(n, star_step(n, secondary)))
            }
            Self::Rose => {
                let divisor = secondary.pick(&rose_divisors(n)).unwrap_or(1);
                ShapeParams::Rose(Rose::new(n, divisor, taper(arity, &range, 22.0, 15.0)))
            }
            Self::SpirographInside => {
                spirograph(n, secondary, taper(arity, &range, 24.0, 17.0), Roll::Inside)
            }
            Self::SpirographOutside => spirograph(
                n,
                secondary,
                taper(arity, &range, 24.0, 17.0),
                Roll::Outside,
            ),
            Self::Guilloche => {
                let (fixed, rolling, pen) = secondary
                    .pick(GUILLOCHE_BASES)
                    .unwrap_or(GUILLOCHE_BASES[0]);
                let width = taper(arity, &range, 22.0, 11.0);
                ShapeParams::Guilloche(Guilloche::new(
                    Spirograph::new(fixed, rolling, pen, Roll::Inside, width),
                    n,
                ))
            }
            Self::Lissajous => {
                let b = secondary.pick(&lissajous_partners(n)).unwrap_or(1);
                ShapeParams::Lissajous(Lissajous::new(
                    n,
                    b,
                    LISSAJOUS_PHASE,
                    taper(arity, &range, 22.0, 12.0),
                ))
            }
            Self::MaurerRose => {
                let step = secondary.pick(MAURER_STEPS).unwrap_or(MAURER_STEPS[0]);
                ShapeParams::MaurerRose(MaurerRose::new(n, step, MAURER_WIDTH))
            }
            Self::Harmonograph => harmonograph_preset(n),
            Self::CycloidRosette => {
                let kind = secondary
                    .pick(&CYCLOID_KINDS)
                    .unwrap_or(CycloidKind::Astroid);
                ShapeParams::CycloidRosette(CycloidRosette::new(
                    kind,
                    n,
                    taper(arity, &range, 20.0, 9.0),
                ))
            }
            Self::Phyllotaxis => {
                ShapeParams::Phyllotaxis(Phyllotaxis::new(n, PHYLLOTAXIS_DOT / (n as f64).sqrt()))
            }
            Self::ModularChords => ShapeParams::ModularChords(ModularChords::new(
                n,
                modular_multiplier(n, secondary),
                taper(arity, &range, 11.0, 5.0).round() as u32,
            )),
            Self::StringArt => ShapeParams::StringArt(StringArt::new(
                secondary.pick_in(4..=10),
                n,
                taper(arity, &range, 16.0, 5.0),
            )),
            Self::TwistRings => {
                let sides = secondary.pick_in(3..=16);
                let corner_pitch = TAU / sides as f64;
                ShapeParams::TwistRings(TwistRings::new(
                    sides,
                    n,
                    TWIST_FRACTION * corner_pitch / n as f64,
                    taper(arity, &range, 22.0, 11.0),
                ))
            }
            Self::MoireRings => ShapeParams::MoireRings(MoireRings::new(
                n,
                secondary.pick(&ring_offsets(n)).unwrap_or(1),
                taper(arity, &range, 20.0, 9.0),
            )),
            Self::MoireGrid => ShapeParams::MoireBars(MoireBars::new(
                MoireKind::Grid,
                n,
                MOIRE_GRID_TWIST / n as f64,
            )),
            Self::MoireWeave => ShapeParams::MoireBars(MoireBars::new(
                MoireKind::Weave,
                n,
                MOIRE_WEAVE_TWIST / n as f64,
            )),
            Self::PinwheelNest => ShapeParams::PinwheelNest(pinwheel_nest(n)),
            Self::StarLattice => {
                let reach = secondary.between(0.55, 0.85);
                ShapeParams::StarLattice(StarLattice::new(
                    n,
                    STAR_LATTICE_WIDTH / (n as f64 * reach),
                    reach,
                ))
            }
            Self::Truchet => ShapeParams::Truchet(Truchet::new(n, TRUCHET_WIDTH / n as f64)),
            Self::ConcentricRings => {
                ShapeParams::ConcentricRings(ConcentricRings::new(nested_radii(n)))
            }
            Self::PolygonRings => ShapeParams::PolygonRings(PolygonRings::new(
                secondary.pick_in(3..=12),
                nested_radii(n),
            )),
            Self::PetalMandala => {
                let spacing = 2.0 * PETAL_RING_RADIUS * (std::f64::consts::PI / n as f64).sin();
                let overlap = secondary.between(PETAL_OVERLAP.0, PETAL_OVERLAP.1);
                ShapeParams::PetalMandala(PetalMandala::new(
                    n,
                    PETAL_RING_RADIUS,
                    overlap * spacing,
                ))
            }
            Self::Slats => {
                let direction = if secondary.get() < 0.5 {
                    bars::Direction::Horizontal
                } else {
                    bars::Direction::Vertical
                };
                ShapeParams::Slats(Slats::new(n, direction, SLAT_DUTY))
            }
            Self::Grid => ShapeParams::Grid(Grid::new(n, GRID_DUTY)),
            Self::Frames => ShapeParams::Frames(Frames::new(n, FRAME_THICKNESS / n as f64)),
        }
    }
}

/// The chord step for a star polygon, in `[3, (n−1)/2]` and coprime with `n`.
///
/// A step sharing a factor with the point count closes the walk early and
/// retraces it, which an even-odd fill cancels to nothing.
fn star_step(points: u32, secondary: Secondary) -> u32 {
    let steps: Vec<u32> = (3..=(points.saturating_sub(1)) / 2)
        .filter(|&k| gcd(k, points) == 1)
        .collect();
    secondary.pick(&steps).unwrap_or(2)
}

/// Divisors a rose may carry.
///
/// The period is computed for a frequency in lowest terms, so a divisor sharing
/// a factor with the petal count sends the curve round its own figure more than
/// once — an even number of times cancels it entirely. A divisor equal to the
/// petal count is a circle through the origin, which the coprimality rule
/// already excludes for every count above one.
fn rose_divisors(petals: u32) -> Vec<u32> {
    (1..=8)
        .filter(|&d| d != petals && gcd(d, petals) == 1)
        .collect()
}

/// Frequencies the other axis of a Lissajous figure may carry.
///
/// Equal frequencies trace an ellipse, which a ring of marks already makes. A
/// frequency sharing a factor with the first closes the figure before the full
/// turn is out and traces it again, which an even number of times cancels.
fn lissajous_partners(a: u32) -> Vec<u32> {
    (1..=9).filter(|&b| b != a && gcd(a, b) == 1).collect()
}

/// A quarter turn is avoided: where `a` is even and `b` odd it traces the figure
/// twice, so the phase is held a third of a turn away from it.
const LISSAJOUS_PHASE: f64 = FRAC_PI_3;

/// The one width the curated Maurer roses carry.
const MAURER_WIDTH: f64 = 5.0;

/// Steps a Maurer walk may advance by.
///
/// The walk closes after `360 / gcd(step, 360)` points, so a step sharing a
/// factor with 360 lands on few angles at nearly equal radii and collapses the
/// figure into a chevron. These are the curated steps that are coprime with 360.
const MAURER_STEPS: &[u32] = &[19, 43, 47, 53, 67, 71, 97, 109, 113, 127];

/// Base curves a guilloche may be built on.
///
/// The base is a whole spirograph rather than a number, so it indexes the
/// curated set rather than being computed.
const GUILLOCHE_BASES: &[(u32, u32, u32)] = &[
    (5, 2, 3),
    (7, 3, 4),
    (9, 4, 5),
    (9, 4, 7),
    (11, 4, 8),
    (12, 5, 7),
    (13, 5, 9),
    (14, 5, 8),
    (15, 4, 9),
    (16, 7, 10),
    (17, 6, 12),
];

/// Offsets a second ring stack may be pitched at.
///
/// The stacks divide the same radius, so an offset sharing a factor with the
/// first count puts a ring of one stack exactly on a ring of the other, and the
/// pair cancels.
fn ring_offsets(rings: u32) -> Vec<u32> {
    (1..=3).filter(|&offset| gcd(rings, offset) == 1).collect()
}

const CYCLOID_KINDS: [CycloidKind; 4] = [
    CycloidKind::Astroid,
    CycloidKind::Deltoid,
    CycloidKind::Cardioid,
    CycloidKind::Nephroid,
];

/// Floret radius against floret count: fitted across sixteen curated figures,
/// which spread from 323 to 341 excluding two deliberate outliers.
const PHYLLOTAXIS_DOT: f64 = 330.0;

/// Relative angle against bar count, fitted across eleven curated grids.
const MOIRE_GRID_TWIST: f64 = 1.8;

/// Relative angle against bar count, fitted across nine curated weaves.
const MOIRE_WEAVE_TWIST: f64 = 1.55;

/// Ribbon width against tile count, fitted across the curated tilings.
const TRUCHET_WIDTH: f64 = 85.0;

/// Total twist across the stack, as a fraction of one corner pitch.
///
/// The curated stacks spread from 0.29 to 0.47 of a pitch.
const TWIST_FRACTION: f64 = 0.4;

/// Ribbon width against tiles and reach.
///
/// The curated lattices spread from 17 to 26 on this product, with no tighter
/// law in them, so this is the middle of the observed band.
const STAR_LATTICE_WIDTH: f64 = 22.0;

/// The radius the petals of a mandala are centred on.
const PETAL_RING_RADIUS: f64 = 260.0;

/// Petal radius as a multiple of the distance between neighbouring petals.
///
/// This is the family's second degree of freedom: at the low end the petals
/// barely meet, at the high end they cut deeply into one another. The three
/// curated mandalas sit at 0.77, 1.00 and 1.09, which is the band.
const PETAL_OVERLAP: (f64, f64) = (0.77, 1.09);

/// Half bar, half gap.
const SLAT_DUTY: f64 = 0.5;

/// Bars thinner than their gaps, so the crossings leave a lattice of holes.
const GRID_DUTY: f64 = 0.35;

/// Border thickness against frame count. Both curated figures sit on this
/// exactly: four frames at 45, six at 30.
const FRAME_THICKNESS: f64 = 180.0;

/// The outermost radius a nested stack starts from.
const OUTER_RADIUS: f64 = 470.0;

/// Radii for a stack of nested rings, evenly divided from the frame inwards.
fn nested_radii(count: u32) -> Vec<f64> {
    (0..count)
        .map(|i| OUTER_RADIUS * (count - i) as f64 / count as f64)
        .collect()
}

/// A spirograph on a chosen lobe count.
///
/// The lobe count is `R / gcd(R, r)`, so driving it from one number means
/// choosing a parameterisation rather than deriving one: the fixed radius is
/// the lobe count itself, and the rolling radius is the whole number nearest
/// two fifths of it that shares no factor with it, so the greatest common
/// divisor is one and the lobes come out exactly. The curated figures cluster
/// at ratios of 0.36 to 0.44 and pen depths of 1.4 to 2.5, so both bands are
/// observed rather than guessed.
fn spirograph(lobes: u32, secondary: Secondary, width: f64, roll: Roll) -> ShapeParams {
    let rolling = coprime_near(lobes, 0.4);
    let depth = secondary.between(1.0, 3.0);
    let pen = ((rolling as f64 * depth).round() as u32).max(1);
    ShapeParams::Spirograph(Spirograph::new(lobes, rolling, pen, roll, width))
}

/// The whole number nearest `fraction` of `n` that shares no factor with it.
fn coprime_near(n: u32, fraction: f64) -> u32 {
    let target = (n as f64 * fraction).round() as u32;
    (1..n)
        .filter(|&r| gcd(r, n) == 1)
        .min_by_key(|&r| (r.abs_diff(target), r))
        .unwrap_or(1)
}

/// The multiplier a modular chord figure follows.
///
/// Where the multiplier squared is one modulo the point count the map pairs `i`
/// with `k·i` and back again, so every chord is drawn twice and the figure
/// cancels against itself entirely.
fn modular_multiplier(points: u32, secondary: Secondary) -> u32 {
    let candidates: Vec<u32> = (2..points.saturating_sub(1))
        .filter(|&k| (k as u64 * k as u64) % points as u64 != 1)
        .collect();
    secondary.pick(&candidates).unwrap_or(2)
}

/// Ribbon width against blade count, per layer, fitted across the curated nests.
const PINWHEEL_WIDTHS: [f64; 3] = [110.0, 135.0, 150.0];

/// Where each layer starts and ends, and how far its blades turn between.
const PINWHEEL_LAYERS: [(f64, f64, f64); 3] =
    [(50.0, 205.0, 2.6), (185.0, 335.0, 2.1), (315.0, 470.0, 1.7)];

/// A nest of blade rings whose counts share no factor.
///
/// One number cannot set a tuple, so the arity is the outermost blade count and
/// the inner counts are the largest preceding values coprime with everything
/// outside them — a chosen mapping, but the one that keeps the coprimality the
/// figure is made of.
fn pinwheel_nest(outermost: u32) -> PinwheelNest {
    let mut counts = vec![outermost];
    while counts.len() < PINWHEEL_LAYERS.len() {
        let smallest = *counts.last().unwrap_or(&outermost);
        match (2..smallest)
            .rev()
            .find(|&c| counts.iter().all(|&k| gcd(c, k) == 1))
        {
            Some(count) => counts.push(count),
            None => break,
        }
    }
    counts.reverse();
    let layers = counts
        .iter()
        .enumerate()
        .map(|(i, &blades)| {
            let (inner, outer, sweep) = PINWHEEL_LAYERS[i];
            pinwheel::Layer::new(
                blades,
                inner,
                outer,
                sweep,
                PINWHEEL_WIDTHS[i] / blades as f64,
            )
        })
        .collect();
    PinwheelNest::new(layers)
}

/// A harmonograph carries no count, so the control selects among the curated
/// figures rather than resolving into a number.
fn harmonograph_preset(index: u32) -> ShapeParams {
    let curated = crate::presets::of_family(ShapeFamily::Harmonograph);
    let i = (index as usize)
        .saturating_sub(1)
        .min(curated.len().saturating_sub(1));
    curated
        .get(i)
        .cloned()
        .unwrap_or(ShapeParams::Harmonograph(FALLBACK_HARMONOGRAPH))
}

/// Stands in if the curated set is ever empty, so a live control still reaches
/// a figure.
const FALLBACK_HARMONOGRAPH: Harmonograph = Harmonograph {
    x: harmonograph::Axis {
        frequencies: (2.0, 2.01),
        damping: (0.012, 0.010),
        phase: 0.0,
    },
    y: harmonograph::Axis {
        frequencies: (3.0, 3.01),
        damping: (0.011, 0.013),
        phase: 1.1,
    },
    turns: 30.0,
    steps: 3000,
    width: 7.0,
};
