//! The two controls, swept across every family.
//!
//! An arity is held rather than animated, but it is still reached live, so every
//! value in a family's range has to give a figure — and it has to give one the
//! even-odd fill does not cancel. The guards the families are built around are
//! checked here at every arity rather than at the handful the curated presets
//! happen to sit on.

use tunnels_shapes::arity::{Arity, Secondary};
use tunnels_shapes::families::{ShapeFamily, ShapeParams};
use tunnels_shapes::geom::{CENTER, EXTENT, Figure, Point};

/// Positions across the secondary control, including both ends.
const SECONDARIES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// How far apart two coordinates may sit and still be the same edge.
///
/// A frame spans a thousand units and is projected onto about a thousand lines,
/// so a thousandth of a unit is a millionth of the figure and far below
/// anything an eye or a rasteriser resolves. It is the slack a pitch divided by
/// a count that does not divide the frame evenly leaves behind, not a tolerance
/// on the property itself.
const COINCIDENT: f64 = 1e-3;

fn sweep(mut visit: impl FnMut(ShapeFamily, Arity, ShapeParams)) {
    for family in ShapeFamily::ALL {
        for n in family.arity_range() {
            let arity = Arity::new(n);
            for s in SECONDARIES {
                visit(family, arity, family.resolve(arity, Secondary::new(s)));
            }
        }
    }
}

#[test]
fn every_arity_gives_a_figure() {
    sweep(|family, arity, params| {
        let figure = params.generate();
        assert!(
            !figure.contours.is_empty(),
            "{} at arity {} generated nothing",
            family.name(),
            arity.get()
        );
        for contour in &figure.contours {
            assert!(
                contour.points().len() > 2,
                "{} at arity {} generated a contour of {} points",
                family.name(),
                arity.get(),
                contour.points().len()
            );
            for p in contour.points() {
                assert!(
                    p.x.is_finite() && p.y.is_finite(),
                    "{} at arity {} generated a coordinate that is not finite",
                    family.name(),
                    arity.get()
                );
            }
        }
    });
}

/// Every way a figure built this way can cancel itself to nothing, checked at
/// every arity the control can reach.
#[test]
fn nothing_the_controls_reach_cancels_itself() {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }

    sweep(|family, arity, params| {
        let at = format!("{} at arity {}", family.name(), arity.get());
        match params {
            // A step sharing a factor with the point count closes the walk
            // early and retraces it.
            ShapeParams::StarPolygon(p) => {
                assert_eq!(gcd(p.step, p.points), 1, "{at}: the walk retraces itself");
            }
            // One petal is a circle through the origin, and a frequency not in
            // lowest terms is traced more than once.
            ShapeParams::Rose(p) => {
                assert_ne!(p.petals, p.divisor, "{at}: the rose is a circle");
                assert_eq!(gcd(p.petals, p.divisor), 1, "{at}: the rose is retraced");
            }
            // Equal frequencies trace an ellipse, and frequencies sharing a
            // factor close early and trace the figure again.
            ShapeParams::Lissajous(p) => {
                assert_ne!(p.a, p.b, "{at}: the figure is an ellipse");
                assert_eq!(gcd(p.a, p.b), 1, "{at}: the figure is retraced");
            }
            // A multiplier that is its own inverse draws every chord twice.
            ShapeParams::ModularChords(p) => {
                let k = p.multiplier as u64;
                assert_ne!(
                    (k * k) % p.points as u64,
                    1,
                    "{at}: every chord is drawn twice"
                );
            }
            // A step sharing a factor with 360 lands on few angles at nearly
            // equal radii, and the walk collapses into a chevron.
            ShapeParams::MaurerRose(p) => {
                assert_eq!(
                    gcd(p.step_degrees, 360),
                    1,
                    "{at}: the walk closes after {} points",
                    360 / gcd(p.step_degrees, 360)
                );
            }
            // A ring of one stack landing on a ring of the other cancels the pair.
            ShapeParams::MoireRings(p) => {
                assert_eq!(
                    gcd(p.rings, p.rings + p.offset),
                    1,
                    "{at}: a ring of each stack lands on the same radius"
                );
            }
            // Blade counts sharing a factor make one figure describable by a
            // single count, which is what the nest exists not to be.
            ShapeParams::PinwheelNest(p) => {
                assert!(
                    p.layers.len() > 1,
                    "{at}: one layer is a ring of blades a beam already makes"
                );
                for (i, a) in p.layers.iter().enumerate() {
                    for b in &p.layers[i + 1..] {
                        assert_eq!(
                            gcd(a.blades, b.blades),
                            1,
                            "{at}: {} and {} share a factor",
                            a.blades,
                            b.blades
                        );
                    }
                }
            }
            // A spirograph's lobe count must be the arity it was asked for.
            ShapeParams::Spirograph(p) => {
                assert_eq!(
                    p.lobes(),
                    arity.get(),
                    "{at}: the lobe count is not the arity"
                );
            }
            _ => {}
        }
    });
}

/// The coordinates a figure occupies along one axis, in order and without
/// repeats.
fn edges(figure: &Figure, of: impl Fn(Point) -> f64) -> Vec<f64> {
    let mut values: Vec<f64> = figure
        .contours
        .iter()
        .flat_map(|contour| contour.points())
        .map(|&point| of(point))
        .collect();
    values.sort_by(f64::total_cmp);
    values.dedup_by(|a, b| (*a - *b).abs() < COINCIDENT);
    values
}

/// Where each bar of a run begins and ends across its own width.
///
/// A bar spans the frame along its length whichever way the run is pitched, so
/// the width is the shorter of its two extents and the only one that says where
/// the run sits.
fn widths(figure: &Figure) -> Vec<(f64, f64)> {
    figure
        .contours
        .iter()
        .map(|contour| {
            let extent = |of: fn(&Point) -> f64| {
                let values = contour.points().iter().map(of);
                values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                })
            };
            let across = extent(|p| p.x);
            let down = extent(|p| p.y);
            if across.1 - across.0 < down.1 - down.0 {
                across
            } else {
                down
            }
        })
        .collect()
}

/// Every coordinate has a partner the same distance from the opposite edge.
fn assert_mirrored(values: &[f64], at: &str, axis: &str) {
    for (&low, &high) in values.iter().zip(values.iter().rev()) {
        assert!(
            (low + high - EXTENT).abs() < COINCIDENT,
            "{at}: {low} and {high} are not the same distance from the two {axis} edges"
        );
    }
}

/// A run of bars reads the same from either edge of the frame.
///
/// Pitching bars from one edge runs the pattern out before it reaches the
/// other, and the marks against the two edges then differ: one set is cut in
/// half by the frame where the other stands whole. These families are as much a
/// border as a fill, and a border that disagrees with itself across the figure
/// reads as a fault rather than as pattern.
///
/// Which edge the pattern ends on is the part that differs between them. Slats
/// close on a bar at both ends, so the figure fills the frame exactly and the
/// pattern never trails off into a gap. A grid holds every bar clear of the
/// frame instead, so all four of its borders end in a whole mark and the border
/// reads as hatch.
#[test]
fn a_run_of_bars_reads_the_same_from_either_edge() {
    sweep(|family, arity, params| {
        let bars = matches!(params, ShapeParams::Slats(_) | ShapeParams::Grid(_));
        if !bars {
            return;
        }

        let at = format!("{} at arity {}", family.name(), arity.get());
        let widths = widths(&params.generate());

        let mut sides: Vec<f64> = widths.iter().flat_map(|&(lo, hi)| [lo, hi]).collect();
        sides.sort_by(f64::total_cmp);
        sides.dedup_by(|a, b| (*a - *b).abs() < COINCIDENT);
        assert_mirrored(&sides, &at, "opposite");

        let (near, far) = (sides[0], sides[sides.len() - 1]);
        match params {
            ShapeParams::Slats(_) => assert!(
                near.abs() < COINCIDENT && (far - EXTENT).abs() < COINCIDENT,
                "{at}: the bars run from {near} to {far}, leaving a gap at one end of the frame"
            ),
            _ => assert!(
                near > COINCIDENT && far < EXTENT - COINCIDENT,
                "{at}: the bars run from {near} to {far}, \
                 so the frame cuts the marks at its edge in half"
            ),
        }
    });
}

/// A stack of polygon rings sits in the middle of the frame.
///
/// Placing a polygon by the circle its corners sit on does not centre what is
/// seen: a polygon with an odd number of corners reaches the whole radius at
/// the corner facing one edge and only `cos(pi / sides)` of it at the side
/// facing the other. At three corners that is a quarter of the radius out of
/// true, which is a tenth of the frame. Even corner counts are symmetric by
/// construction and would pass this on their own, so the odd ones are what it
/// is here for.
///
/// Sweeping the second control rather than reading each family's pinned
/// position is what reaches this at all, and the reason is general enough to
/// be worth stating: a construction whose pinned positions all happen to be
/// even-cornered is off centre everywhere and draws nothing off centre, so the
/// fault sits in the library without appearing in any figure the library
/// holds — until a position that was never pinned becomes a family, and every
/// figure of it is wrong. A position nothing selects today is one a later
/// family can be pinned to, so the guards belong on the whole range and not on
/// the table.
#[test]
fn a_stack_of_polygon_rings_is_centred_on_the_frame() {
    sweep(|family, arity, params| {
        let ShapeParams::PolygonRings(ref rings) = params else {
            return;
        };

        let at = format!(
            "{} at arity {} on {} sides",
            family.name(),
            arity.get(),
            rings.sides
        );
        let figure = params.generate();
        for (values, axis) in [
            (edges(&figure, |p| p.x), "across"),
            (edges(&figure, |p| p.y), "down"),
        ] {
            let middle = (values[0] + values[values.len() - 1]) / 2.0;
            assert!(
                (middle - CENTER).abs() < COINCIDENT,
                "{at}: the stack runs from {} to {} {axis}, centred on {middle} \
                 rather than on {CENTER}",
                values[0],
                values[values.len() - 1]
            );
        }
    });
}

/// Two families built by the same construction must draw different figures.
///
/// A family is a construction with its second degree of freedom pinned, so
/// families sharing a construction are told apart by that pin and by nothing
/// else. A pin that resolves to what a sibling's resolves to at every count
/// they both offer spends two positions of the family control on one figure.
#[test]
fn families_sharing_a_construction_draw_different_figures() {
    for (i, &family) in ShapeFamily::ALL.iter().enumerate() {
        for &sibling in &ShapeFamily::ALL[i + 1..] {
            if family.generator() != sibling.generator() {
                continue;
            }
            let shared: Vec<Arity> = family
                .arities()
                .iter()
                .copied()
                .filter(|arity| sibling.arities().contains(arity))
                .collect();
            assert!(
                !shared.is_empty(),
                "{} and {} are the same construction but share no arity",
                family.name(),
                sibling.name()
            );
            assert!(
                shared.iter().any(|&arity| {
                    family.resolve(arity, family.secondary())
                        != sibling.resolve(arity, sibling.secondary())
                }),
                "{} and {} draw the same figure at all {} of the arities they share",
                family.name(),
                sibling.name(),
                shared.len()
            );
        }
    }
}

/// An arity outside a family's range must land inside it rather than escaping.
#[test]
fn arities_outside_the_range_are_brought_back_in() {
    for family in ShapeFamily::ALL {
        let range = family.arity_range();
        for out_of_range in [0, u32::MAX] {
            let params = family.resolve(Arity::new(out_of_range), Secondary::new(0.5));
            let inside = family.resolve(
                Arity::new(out_of_range.clamp(*range.start(), *range.end())),
                Secondary::new(0.5),
            );
            assert_eq!(
                params,
                inside,
                "{} at arity {out_of_range} left its range",
                family.name()
            );
        }
    }
}

/// A family the knob cannot reach, or one with nothing curated in it, is a
/// family that does not exist as far as a show is concerned.
///
/// This does not catch a family left out of `ShapeFamily::ALL` that also has no
/// preset — but a family earns its place by having been curated, so one with no
/// preset is not a family yet.
#[test]
fn every_family_is_reachable() {
    use std::collections::BTreeSet;
    use tunnels_shapes::families::Generator;
    use tunnels_shapes::presets;

    /// Constructions curation kept that the library offers no family of.
    ///
    /// A construction earns a place by having been curated, but curation is
    /// necessary rather than sufficient: these were built, looked at, and left
    /// out. Naming them is what keeps a family dropped on purpose apart from a
    /// family dropped by accident, and what makes deleting one from the table
    /// without saying so a failure here.
    const CULLED: [Generator; 7] = [
        Generator::Lissajous,
        Generator::MaurerRose,
        Generator::Harmonograph,
        Generator::CycloidRosette,
        Generator::PinwheelNest,
        Generator::Truchet,
        Generator::Frames,
    ];

    let families: BTreeSet<ShapeFamily> = ShapeFamily::ALL.into_iter().collect();
    assert_eq!(
        families.len(),
        ShapeFamily::ALL.len(),
        "a family is listed twice in the sweep"
    );

    let offered: BTreeSet<Generator> = families.iter().map(|f| f.generator()).collect();
    let curated: BTreeSet<Generator> = presets::all()
        .iter()
        .map(|preset| preset.params.generator())
        .collect();

    for construction in &offered {
        assert!(
            curated.contains(construction),
            "{}: offered by the family control, but nothing built from it was curated",
            construction.name()
        );
    }
    for construction in &curated {
        assert!(
            offered.contains(construction) || CULLED.contains(construction),
            "{}: curated and not offered, without being named as culled",
            construction.name()
        );
    }
    for construction in CULLED {
        assert!(
            !offered.contains(&construction),
            "{}: named as culled, but the family control offers it",
            construction.name()
        );
    }
}

/// The library table, checked against the ranges the families justify.
///
/// Writing the arities out is what lets curation choose them, and the cost of
/// that is a list that can disagree with the range it is drawn from — a family
/// whose range is later narrowed, or a digit typed wrong. Nothing else notices:
/// `resolve` clamps, so an arity outside the range would quietly draw the
/// figure at the boundary under the wrong name.
#[test]
fn every_offered_arity_is_one_the_family_admits() {
    /// The most positions one knob is asked to select between.
    const CAP: usize = 16;

    let mut total = 0;
    for family in ShapeFamily::ALL {
        let range = family.arity_range();
        let arities = family.arities();
        total += arities.len();

        assert!(
            !arities.is_empty() && arities.len() <= CAP,
            "{} offers {} arities, which is not between one and {CAP}",
            family.name(),
            arities.len()
        );
        for arity in arities {
            assert!(
                range.contains(&arity.get()),
                "{} offers arity {}, which is outside {range:?}",
                family.name(),
                arity.get()
            );
        }
        assert!(
            arities.windows(2).all(|pair| pair[0] < pair[1]),
            "{}'s arities are not in ascending order: {arities:?}",
            family.name()
        );
    }
    assert_eq!(total, 514, "the generated library is a different size");
}

/// The families whose second control resolves into nothing, named rather than
/// discovered, so that a family gaining a degree of freedom has to say so here.
#[test]
fn only_the_families_without_a_second_freedom_ignore_the_secondary() {
    const WITHOUT: [ShapeFamily; 6] = [
        ShapeFamily::Phyllotaxis,
        ShapeFamily::MoireGrid,
        ShapeFamily::MoireWeave,
        ShapeFamily::ConcentricRings,
        ShapeFamily::Slats,
        ShapeFamily::Grid,
    ];

    for family in ShapeFamily::ALL {
        let unmoved = family.arities().iter().all(|&arity| {
            let pinned = family.resolve(arity, family.secondary());
            SECONDARIES
                .iter()
                .all(|&s| family.resolve(arity, Secondary::new(s)) == pinned)
        });
        assert_eq!(
            unmoved,
            WITHOUT.contains(&family),
            "{} {} its secondary",
            family.name(),
            if unmoved { "ignores" } else { "resolves" }
        );
    }
}
