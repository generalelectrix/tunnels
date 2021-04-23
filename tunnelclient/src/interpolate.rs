//! Perform linear interpolation between entities.

use std::sync::Arc;

use interpolation::lerp;
use tunnels_lib::ArcSegment;
use tunnels_lib::{min_included_angle, modulo};

/// Allow an entity to be interpolated with another instance of Self.
pub trait Interpolate {
    /// Perform interpolation between self and other given easing parameter alpha on [0.0, 1.0].
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self;
}

impl<T: Interpolate + Clone> Interpolate for Vec<T> {
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self {
        if self.len() != other.len() {
            if alpha < 0.5 {
                return self.clone();
            } else {
                return other.clone();
            }
        }
        self.iter()
            .zip(other.iter())
            .map(|(a, b)| a.interpolate_with(b, alpha))
            .collect::<Vec<_>>()
    }
}

impl<T: Interpolate> Interpolate for Arc<T> {
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self {
        Arc::new((**self).interpolate_with(other, alpha))
    }
}

/// Interpolate a pytunnel-style unit angle.
/// Ensure that we always interpolate along the shortest path between the two angular coordinates
/// that we are easing between.
#[inline(always)]
fn interpolate_angle(a: f64, b: f64, alpha: f64) -> f64 {
    let shortest_angle = min_included_angle(a, b);
    modulo(a + shortest_angle * alpha, 1.0)
}

impl Interpolate for ArcSegment {
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self {
        ArcSegment {
            level: lerp(&self.level, &other.level, &alpha),
            thickness: lerp(&self.thickness, &other.thickness, &alpha),
            hue: interpolate_angle(self.hue, other.hue, alpha),
            sat: lerp(&self.sat, &other.sat, &alpha),
            val: lerp(&self.val, &other.val, &alpha),
            x: lerp(&self.x, &other.x, &alpha),
            y: lerp(&self.y, &other.y, &alpha),
            rad_x: lerp(&self.rad_x, &other.rad_x, &alpha),
            rad_y: lerp(&self.rad_y, &other.rad_y, &alpha),
            start: interpolate_angle(self.start, other.start, alpha),
            stop: interpolate_angle(self.stop, other.stop, alpha),
            rot_angle: interpolate_angle(self.rot_angle, other.rot_angle, alpha),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receive::test::arc_segment_for_test;
    use interpolation::lerp;
    use tunnels_lib::assert_almost_eq;

    #[test]
    fn test_interp_angle() {
        assert_almost_eq(0.0, interpolate_angle(0.0, 0.0, 0.0));
        assert_almost_eq(0.0, interpolate_angle(0.0, 1.0, 0.5));
        assert_almost_eq(0.95, interpolate_angle(0.0, 0.9, 0.5));
        assert_almost_eq(0.0, interpolate_angle(0.2, 0.8, 0.5));
        assert_almost_eq(0.0, interpolate_angle(0.2, 0.8, 0.5));
    }

    #[test]
    fn test_interp_arcs() {
        let a = arc_segment_for_test(0.0, 0.0);
        let b = arc_segment_for_test(1.0, 0.4);
        let halfway = arc_segment_for_test(0.5, 0.2);
        assert_eq!(a, a.interpolate_with(&b, 0.0));
        assert_eq!(b, a.interpolate_with(&b, 1.0));
        assert_eq!(halfway, a.interpolate_with(&b, 0.5));
    }

    impl Interpolate for f64 {
        fn interpolate_with(&self, other: &Self, alpha: f64) -> Self {
            lerp(self, other, &alpha)
        }
    }

    #[test]
    fn test_interp_vec() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 1.0, 1.0];
        let halfway = vec![0.5, 0.5, 0.5];
        assert_eq!(a, a.interpolate_with(&b, 0.0));
        assert_eq!(b, a.interpolate_with(&b, 1.0));
        assert_eq!(halfway, a.interpolate_with(&b, 0.5));
    }
}
