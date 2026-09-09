//! Approximations tuned to what a projector can resolve.
//!
//! `libm` is accurate to well under one ULP, which is far more than geometry
//! needs: an angle here ends up as a position in a colour ramp or a rotation of
//! a vertex, and a mesh triangle spans some fourteen pixels. A profile of the
//! prototype put `atan2f` at a quarter of the frame, so the accuracy is worth
//! trading.

use std::f32::consts::{FRAC_PI_2, PI};

/// Polynomial for `atan` on [-1, 1], odd terms only since `atan` is odd.
///
/// Worst error is 2.1e-4 radians, at the diagonals where the octant reduction
/// hands over. The test measures it rather than trusting this comment.
#[inline]
fn atan_unit(x: f32) -> f32 {
    let x2 = x * x;
    x * (0.999_777_3
        + x2 * (-0.332_623_47
            + x2 * (0.193_543_46 + x2 * (-0.116_432_87 + x2 * (0.052_653_32 + x2 * -0.011_721_2)))))
}

/// `y.atan2(x)`, to the accuracy a rendered angle needs.
///
/// Reduces to the octant where `|y| <= |x|` so the polynomial is only ever
/// evaluated on the interval it was fitted to.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let angle = if ay <= ax {
        atan_unit(y / x).abs()
    } else {
        FRAC_PI_2 - atan_unit(x / y).abs()
    };
    // Reflect back out of the first quadrant.
    let angle = if x < 0.0 { PI - angle } else { angle };
    if y < 0.0 { -angle } else { angle }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The error budget.
    ///
    /// An angle here becomes a position in a colour ramp or a rotation of a
    /// vertex. At this bound a point on the rim of a figure a thousand pixels
    /// across moves by 3e-4 * 500 = 0.15 of a pixel, which no projector
    /// resolves and no mesh triangle — fourteen pixels across — comes near.
    const TOLERANCE: f32 = 3e-4;

    #[test]
    fn the_approximation_holds_across_every_octant() {
        let mut worst: f32 = 0.0;
        let mut worst_at = (0.0, 0.0);
        // A grid dense enough to cross every branch, including both axes and
        // both diagonals where the octant reduction switches arms.
        let steps = 401;
        for i in 0..steps {
            for j in 0..steps {
                let x = -2.0 + 4.0 * i as f32 / (steps - 1) as f32;
                let y = -2.0 + 4.0 * j as f32 / (steps - 1) as f32;
                let mut error = (atan2(y, x) - y.atan2(x)).abs();
                // Either side of the cut at pi is the same direction.
                if error > PI {
                    error = (2.0 * PI - error).abs();
                }
                if error > worst {
                    worst = error;
                    worst_at = (x, y);
                }
            }
        }
        assert!(
            worst < TOLERANCE,
            "worst error {worst} at {worst_at:?}, over {TOLERANCE}"
        );
    }

    #[test]
    fn the_axes_and_the_origin_are_exact_enough() {
        for (y, x, expect) in [
            (0.0, 1.0, 0.0),
            (1.0, 0.0, FRAC_PI_2),
            (0.0, -1.0, PI),
            (-1.0, 0.0, -FRAC_PI_2),
            (0.0, 0.0, 0.0),
        ] {
            let got: f32 = atan2(y, x);
            assert!(
                (got - expect).abs() < TOLERANCE,
                "atan2({y}, {x}) = {got}, expected {expect}"
            );
        }
    }
}
