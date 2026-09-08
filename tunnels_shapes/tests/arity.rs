//! The two controls, swept across every family.
//!
//! An arity is held rather than animated, but it is still reached live, so every
//! value in a family's range has to give a figure — and it has to give one the
//! even-odd fill does not cancel. The guards the families are built around are
//! checked here at every arity rather than at the handful the curated presets
//! happen to sit on.

use tunnels_shapes::arity::{Arity, Secondary};
use tunnels_shapes::families::{ShapeFamily, ShapeParams};

/// Positions across the secondary control, including both ends.
const SECONDARIES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

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
            // One petal is a circle through the origin.
            ShapeParams::Rose(p) => {
                assert_ne!(p.petals, p.divisor, "{at}: the rose is a circle");
            }
            // Equal frequencies trace an ellipse.
            ShapeParams::Lissajous(p) => {
                assert_ne!(p.a, p.b, "{at}: the figure is an ellipse");
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

/// Both ends of the secondary control must reach different figures, or the
/// control is not a control.
#[test]
fn the_secondary_control_does_something() {
    for family in ShapeFamily::ALL {
        // These families carry a single degree of freedom: everything else about
        // them follows from arity, or, for the harmonograph, from a curated set
        // there is nothing to compute.
        if matches!(
            family,
            ShapeFamily::Phyllotaxis
                | ShapeFamily::MoireGrid
                | ShapeFamily::MoireWeave
                | ShapeFamily::Truchet
                | ShapeFamily::ConcentricRings
                | ShapeFamily::Grid
                | ShapeFamily::Frames
                | ShapeFamily::PinwheelNest
                | ShapeFamily::Harmonograph
        ) {
            continue;
        }
        let arity = Arity::new(*family.arity_range().end());
        let low = family.resolve(arity, Secondary::new(0.0));
        let high = family.resolve(arity, Secondary::new(1.0));
        assert_ne!(
            low,
            high,
            "{}: both ends of the secondary reach the same figure",
            family.name()
        );
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
