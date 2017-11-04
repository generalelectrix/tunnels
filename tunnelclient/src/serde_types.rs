// deserialization structs
use utils::{almost_eq, angle_almost_eq};

#[derive(Deserialize, Debug, Clone)]
pub struct ArcSegment {
    pub level: f64,
    pub thickness: f64,
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    pub x: f64,
    pub y: f64,
    pub rad_x: f64,
    pub rad_y: f64,
    pub start: f64,
    pub stop: f64,
    pub rot_angle: f64
}

impl ArcSegment {
    /// Return an arc segment for testing, with all linear coordinates set to
    /// linear, and all radial coordinates set to radial.
    pub fn for_test(linear: f64, radial: f64) -> Self {
        ArcSegment{
            level: linear,
            thickness: linear,
            sat: linear,
            val: linear,
            x: linear,
            y: linear,
            rad_x: linear,
            rad_y: linear,
            // radial items
            hue: radial,
            start: radial,
            stop: radial,
            rot_angle: radial
        }
    }
}

impl PartialEq for ArcSegment {
    fn eq(&self, o: &Self) -> bool {
        almost_eq(self.level, o.level) &&
        almost_eq(self.thickness, o.thickness) &&
        almost_eq(self.sat, o.sat) &&
        almost_eq(self.val, o.val) &&
        almost_eq(self.x, o.x) &&
        almost_eq(self.y, o.y) &&
        almost_eq(self.rad_x, o.rad_x) &&
        almost_eq(self.rad_y, o.rad_y) &&
        angle_almost_eq(self.hue, o.hue) &&
        angle_almost_eq(self.start, o.start) &&
        angle_almost_eq(self.stop, o.stop) &&
        angle_almost_eq(self.rot_angle, o.rot_angle)
    }
}

impl Eq for ArcSegment {}

pub type LayerCollection = Vec<Vec<ArcSegment>>;

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub frame_number: u64,
    pub time: u64, // ms
    pub layers: LayerCollection
}

impl Eq for Snapshot {}

mod tests {
    use super::*;

    #[test]
    fn test_arc_eq() {
        let a = ArcSegment::for_test(1.0, 0.5);
        let b = ArcSegment::for_test(0.4, 0.5);
        assert_eq!(a, a);
        assert!(a != b);
    }
}