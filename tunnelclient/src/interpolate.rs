use interpolation::lerp;
use receive::{ArcSegment};
use traits::Interpolate;

/// Interpolate a pytunnel-style unit angle.
#[inline(always)]
fn interpolate_angle(a: f64, b: f64, alpha: f64) -> f64 {
    let shortest_angle = ((((b - a) % 1.0) + 1.5) % 1.0) - 0.5;
    return shortest_angle * alpha;
}

impl Interpolate for ArcSegment {
    fn interpolate_with(&self, other: &Self, alpha: f64) -> Self {
        ArcSegment{
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
            rot_angle: interpolate_angle(self.rot_angle, other.rot_angle, alpha)
        }
    }
}