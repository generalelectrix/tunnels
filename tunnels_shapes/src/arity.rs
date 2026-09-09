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
use std::f64::consts::TAU;
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
    /// Every family, in the order the enum declares them, which is the order a
    /// family control walks.
    ///
    /// Families sharing a construction sit together, so turning the knob past
    /// one of them lands on its siblings before it reaches anything else. The
    /// polygon rings run up through their side counts and hand straight over to
    /// the circle, so the knob walks seven-sided, then round.
    ///
    /// A new family has to be added here as well as to the enum. The compiler
    /// catches a family missing from `name`, `generator` and `resolve`, which
    /// are exhaustive matches, but not one missing from this list;
    /// `every_family_is_reachable` is what catches that.
    pub const ALL: [ShapeFamily; 37] = [
        Self::StarPolygon,
        Self::Rose1,
        Self::Rose2,
        Self::Rose3,
        Self::SpirographInside1,
        Self::SpirographInside2,
        Self::SpirographInside3,
        Self::SpirographOutside1,
        Self::SpirographOutside2,
        Self::SpirographOutside3,
        Self::Guilloche,
        Self::Phyllotaxis,
        Self::ModularChords1,
        Self::ModularChords2,
        Self::ModularChords3,
        Self::StringArt4,
        Self::StringArt5,
        Self::StringArt6,
        Self::TwistRings3,
        Self::TwistRings4,
        Self::MoireRings,
        Self::MoireGrid,
        Self::MoireWeave,
        Self::StarLattice55,
        Self::StarLattice70,
        Self::StarLattice85,
        Self::PolygonRings3,
        Self::PolygonRings4,
        Self::PolygonRings5,
        Self::PolygonRings6,
        Self::PolygonRings7,
        Self::ConcentricRings,
        Self::PetalMandala1,
        Self::PetalMandala2,
        Self::PetalMandala3,
        Self::Slats,
        Self::Grid,
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
            Self::Rose1 | Self::Rose2 | Self::Rose3 => 2..=13,
            // Two lobes is an ellipse.
            Self::SpirographInside1
            | Self::SpirographInside2
            | Self::SpirographInside3
            | Self::SpirographOutside1
            | Self::SpirographOutside2
            | Self::SpirographOutside3 => 3..=21,
            // One copy is a plain spirograph with nothing to interfere with,
            // and past six the centre fills in with a grey knot.
            Self::Guilloche => 2..=6,
            // Below about twenty florets the spiral is a scatter of dots.
            Self::Phyllotaxis => 24..=400,
            // Below about twenty points the chords outline a polygon.
            Self::ModularChords1 | Self::ModularChords2 | Self::ModularChords3 => 24..=220,
            Self::StringArt4 | Self::StringArt5 | Self::StringArt6 => 5..=34,
            // Two rings, below which a relative turn registers against nothing.
            Self::TwistRings3 | Self::TwistRings4 => 3..=20,
            // Too few of either and there is no second pattern to beat against.
            Self::MoireRings => 3..=26,
            // The upper end is where the bars merge into an even grey rather
            // than beating against each other, which is a viewing distance
            // rather than a geometric bound. A weave carries two marks per
            // crossing where a grid carries one, so it merges sooner.
            Self::MoireGrid => 8..=24,
            Self::MoireWeave => 6..=20,
            Self::StarLattice55 | Self::StarLattice70 | Self::StarLattice85 => 1..=5,
            // One ring is a plain polygon, which is still a figure a beam
            // cannot draw.
            Self::PolygonRings3
            | Self::PolygonRings4
            | Self::PolygonRings5
            | Self::PolygonRings6
            | Self::PolygonRings7 => 1..=16,
            Self::ConcentricRings
            | Self::PetalMandala1
            | Self::PetalMandala2
            | Self::PetalMandala3
            | Self::Slats
            | Self::Grid => 4..=16,
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
            Self::Rose1 | Self::Rose2 | Self::Rose3 => {
                arities![2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
            }
            // Every lobe count to fifteen, then every other one: the curated
            // spirographs stop at seventeen inside and outside, and two lobes
            // apart is where a count still reads as a different figure.
            Self::SpirographInside1
            | Self::SpirographInside2
            | Self::SpirographInside3
            | Self::SpirographOutside1
            | Self::SpirographOutside2
            | Self::SpirographOutside3 => {
                arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 19, 21]
            }
            Self::Guilloche => arities![2, 3, 4, 5, 6],
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
            Self::ModularChords1 | Self::ModularChords2 | Self::ModularChords3 => {
                arities![
                    24, 30, 36, 45, 48, 56, 60, 90, 96, 120, 128, 144, 150, 180, 200, 220
                ]
            }
            // The sixteen curated corner counts exactly.
            Self::StringArt4 | Self::StringArt5 | Self::StringArt6 => {
                arities![5, 6, 7, 8, 9, 10, 12, 13, 14, 16, 18, 20, 22, 26, 30, 34]
            }
            Self::TwistRings3 | Self::TwistRings4 => {
                arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20]
            }
            Self::MoireRings => arities![3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 16, 19, 22, 24, 26],
            // Every count the range admits, which is what a beat pattern wants:
            // one bar more is a different figure here where elsewhere it would
            // be the same one drawn slightly finer.
            Self::MoireGrid => {
                arities![8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24]
            }
            Self::MoireWeave => {
                arities![6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
            }
            Self::StarLattice55 | Self::StarLattice70 | Self::StarLattice85 => {
                arities![1, 2, 3, 4, 5]
            }
            Self::PolygonRings3
            | Self::PolygonRings4
            | Self::PolygonRings5
            | Self::PolygonRings6
            | Self::PolygonRings7 => {
                arities![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
            }
            Self::ConcentricRings
            | Self::PetalMandala1
            | Self::PetalMandala2
            | Self::PetalMandala3
            | Self::Slats
            | Self::Grid => arities![4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        }
    }

    /// The second degree of freedom every figure of this family is drawn at.
    ///
    /// One position rather than a range, because the two controls a figure mode
    /// has are spent on the family and the count within it. A family is this
    /// position together with a construction, so the position is what
    /// distinguishes families sharing one.
    ///
    /// Each of these is a position someone looked at a figure at and kept,
    /// rather than a position computed from anything. That is the authority for
    /// the number: which figure of a family is worth a knob position is a
    /// matter of taste, and the evidence about taste is a person looking at
    /// figures. Where a position resolves to something with a name — a
    /// corner count, a pen depth, a reach — it is given here, so a position
    /// that stops resolving to what it says is visible as a disagreement
    /// between this comment and its number.
    ///
    /// Six families resolve nothing from this and take zero: `phyllo`,
    /// `moire_grid`, `moire_weave`, `rings`, `slats` and `grid`.
    pub fn secondary(self) -> Secondary {
        Secondary::new(match self {
            // A step near two fifths of the point count.
            Self::StarPolygon => 0.76,
            // A divisor of one, which is the plain rose; then the low divisors;
            // then a divisor of seven, which laces the petals over each other.
            //
            // The first two coincide at six, ten and twelve petals. A divisor
            // has to be coprime with the petal count, which leaves only three
            // candidates at those three counts, and both positions then select
            // the first of the three. So three of this construction's
            // thirty-six figures are drawn twice, and moving the second
            // position off them would move it at every other count as well.
            Self::Rose1 => 0.0,
            Self::Rose2 => 0.25,
            Self::Rose3 => 0.76,
            // Pen depths of 1.00, 1.92 and 2.90. Depth one is the cusped
            // hypocycloid, where a ribbon leaves the tips as detached blobs.
            Self::SpirographInside1 | Self::SpirographOutside1 => 0.0,
            Self::SpirographInside2 | Self::SpirographOutside2 => 0.46,
            Self::SpirographInside3 | Self::SpirographOutside3 => 0.95,
            // The simplest base. A guilloche's marks are the base's lobes
            // times its copies, and the copies are what the other knob turns.
            Self::Guilloche => 0.0,
            // A multiplier of two, which is the cardioid and the one figure of
            // this family that is not symmetric about an axis; then a middling
            // multiplier; then one near the point count.
            Self::ModularChords1 => 0.0,
            Self::ModularChords2 => 0.5,
            Self::ModularChords3 => 0.99,
            // Four, five and six corners.
            Self::StringArt4 => 0.07,
            Self::StringArt5 => 0.21,
            Self::StringArt6 => 0.36,
            // Three sides and four.
            Self::TwistRings3 => 0.04,
            Self::TwistRings4 => 0.11,
            // Two rings of offset, which beats coarsely enough to read as
            // separated bands rather than as an even grey at the top of the
            // range. Two and three share a factor with 6 and with 24, and at
            // those two counts the family falls back to an offset of one and
            // draws the finer figure — a discontinuity at two of the sixteen
            // positions, taken over fourteen that would otherwise be haze.
            Self::MoireRings => 0.5,
            // Reaches of 0.55, 0.70 and 0.85.
            Self::StarLattice55 => 0.0,
            Self::StarLattice70 => 0.5,
            Self::StarLattice85 => 1.0,
            // Three sides through seven. A stack of circles is its own family,
            // and an octagon is nearly one.
            Self::PolygonRings3 => 0.05,
            Self::PolygonRings4 => 0.15,
            Self::PolygonRings5 => 0.25,
            Self::PolygonRings6 => 0.35,
            Self::PolygonRings7 => 0.45,
            // Petal radii of 0.80, 0.90 and the whole ring radius, at which
            // every petal's rim passes through the centre.
            Self::PetalMandala1 => 0.0,
            Self::PetalMandala2 => 0.5,
            Self::PetalMandala3 => 1.0,
            Self::Phyllotaxis
            | Self::MoireGrid
            | Self::MoireWeave
            | Self::ConcentricRings
            | Self::Slats
            | Self::Grid => 0.0,
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
            Self::Rose1 | Self::Rose2 | Self::Rose3 => {
                let divisor = secondary.pick(&rose_divisors(n)).unwrap_or(1);
                ShapeParams::Rose(Rose::new(n, divisor, taper(arity, &range, 22.0, 15.0)))
            }
            Self::SpirographInside1 | Self::SpirographInside2 | Self::SpirographInside3 => {
                spirograph(n, secondary, taper(arity, &range, 24.0, 17.0), Roll::Inside)
            }
            Self::SpirographOutside1 | Self::SpirographOutside2 | Self::SpirographOutside3 => {
                spirograph(
                    n,
                    secondary,
                    taper(arity, &range, 24.0, 17.0),
                    Roll::Outside,
                )
            }
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
            Self::Phyllotaxis => {
                ShapeParams::Phyllotaxis(Phyllotaxis::new(n, PHYLLOTAXIS_DOT / (n as f64).sqrt()))
            }
            Self::ModularChords1 | Self::ModularChords2 | Self::ModularChords3 => {
                ShapeParams::ModularChords(ModularChords::new(
                    n,
                    modular_multiplier(n, secondary),
                    taper(arity, &range, 11.0, 5.0).round() as u32,
                ))
            }
            Self::StringArt4 | Self::StringArt5 | Self::StringArt6 => {
                ShapeParams::StringArt(StringArt::new(
                    secondary.pick_in(4..=10),
                    n,
                    taper(arity, &range, 16.0, 5.0),
                ))
            }
            Self::TwistRings3 | Self::TwistRings4 => {
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
            Self::StarLattice55 | Self::StarLattice70 | Self::StarLattice85 => {
                let reach = secondary.between(0.55, 0.85);
                ShapeParams::StarLattice(StarLattice::new(
                    n,
                    STAR_LATTICE_WIDTH / (n as f64 * reach),
                    reach,
                ))
            }
            Self::PolygonRings3
            | Self::PolygonRings4
            | Self::PolygonRings5
            | Self::PolygonRings6
            | Self::PolygonRings7 => ShapeParams::PolygonRings(PolygonRings::new(
                secondary.pick_in(3..=12),
                nested_radii(n),
            )),
            Self::ConcentricRings => {
                ShapeParams::ConcentricRings(ConcentricRings::new(nested_radii(n)))
            }
            Self::PetalMandala1 | Self::PetalMandala2 | Self::PetalMandala3 => {
                let reach = secondary.between(PETAL_REACH.0, PETAL_REACH.1);
                ShapeParams::PetalMandala(PetalMandala::new(
                    n,
                    PETAL_RING_RADIUS,
                    reach * PETAL_RING_RADIUS,
                ))
            }
            // Which way the bars run is not a degree of freedom a figure of
            // this family is chosen for, so the family fixes it.
            Self::Slats => {
                ShapeParams::Slats(Slats::new(n, bars::Direction::Horizontal, SLAT_DUTY))
            }
            Self::Grid => ShapeParams::Grid(Grid::new(n, GRID_DUTY)),
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

/// Floret radius against floret count: fitted across sixteen curated figures,
/// which spread from 323 to 341 excluding two deliberate outliers.
const PHYLLOTAXIS_DOT: f64 = 330.0;

/// Relative angle against bar count, fitted across eleven curated grids.
const MOIRE_GRID_TWIST: f64 = 1.8;

/// Relative angle against bar count, fitted across nine curated weaves.
const MOIRE_WEAVE_TWIST: f64 = 1.55;

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

/// Petal radius as a fraction of the radius the petals are centred on.
///
/// This is the family's second degree of freedom, and it is a fraction of the
/// ring rather than of the gap between neighbouring petals so that it means the
/// same thing at every arity. Neighbours close up as the petal count rises, so
/// a radius tied to that gap shrinks with the count and leaves a hole in the
/// middle that grows as the figure gets busier. Tied to the ring, a petal
/// reaches as far in at sixteen petals as at four, and the extra petals overlap
/// each other instead of retreating from the centre.
///
/// At the top of the band every petal's rim passes through the centre of the
/// figure, which is as far as reaching in goes.
const PETAL_REACH: (f64, f64) = (0.80, 1.00);

/// Half bar, half gap.
const SLAT_DUTY: f64 = 0.5;

/// Bars thinner than their gaps, so the crossings leave a lattice of holes.
const GRID_DUTY: f64 = 0.35;

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
