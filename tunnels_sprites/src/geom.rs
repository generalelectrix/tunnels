//! The coordinate system every baked figure is expressed in.
//!
//! The crate holding the procedurally generated figures defines its own
//! equivalents of these types for the same coordinate system. Which of the two
//! survives, or whether both give way to a shared one, is a question for
//! whoever brings the generated and baked halves together — neither side can
//! answer it alone, so the duplication stands until then rather than being
//! resolved by whichever landed second.

/// A point in shape space: the unit box centred on the origin that every
/// figure is normalised into.
///
/// Normalising is what makes a beam's size knob mean the same thing whatever
/// coordinate system the source artwork used, and this type is what says a
/// value is in that space rather than in pixels on a screen.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
#[repr(transparent)]
pub struct Point([f32; 2]);

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self([x, y])
    }

    pub const fn from_array(coords: [f32; 2]) -> Self {
        Self(coords)
    }

    /// The coordinates as a bare pair.
    pub const fn to_array(self) -> [f32; 2] {
        self.0
    }

    pub const fn x(self) -> f32 {
        self.0[0]
    }

    pub const fn y(self) -> f32 {
        self.0[1]
    }

    /// The point halfway between two others.
    ///
    /// Written as the mean of each coordinate, and not as any algebraically
    /// equal alternative: float addition is commutative but most rearrangements
    /// of it are not exact, so this is what makes the result bit-identical
    /// whichever way round the two points are given.
    pub fn midpoint(self, other: Self) -> Self {
        Self([
            (self.0[0] + other.0[0]) / 2.0,
            (self.0[1] + other.0[1]) / 2.0,
        ])
    }

    pub fn distance(self, other: Self) -> f32 {
        ((self.0[0] - other.0[0]).powi(2) + (self.0[1] - other.0[1]).powi(2)).sqrt()
    }

    /// Distance from the origin, which a figure is centred on.
    pub fn radius(self) -> f32 {
        (self.0[0] * self.0[0] + self.0[1] * self.0[1]).sqrt()
    }

    /// Both coordinates as raw bits, for keying on exact identity.
    ///
    /// Points that came out of the same arithmetic agree bit for bit; points
    /// that merely landed close do not, and should not be treated as one.
    pub fn bits(self) -> (u32, u32) {
        (self.0[0].to_bits(), self.0[1].to_bits())
    }
}

/// One closed loop, as the polyline its curves flattened to.
///
/// Closed whether or not the source said so, and never shorter than three
/// points: a fill is an area, and fewer than three points enclose none.
#[derive(Debug, Clone, PartialEq)]
pub struct Contour(Vec<Point>);

impl Contour {
    /// A loop, or `None` from fewer than three points.
    pub fn new(points: Vec<Point>) -> Option<Self> {
        (points.len() > 2).then_some(Self(points))
    }

    pub fn points(&self) -> &[Point] {
        &self.0
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The refinement dedups vertices by value, so both triangles sharing an
    /// edge have to arrive at the same midpoint bit for bit.
    #[test]
    fn a_midpoint_is_the_same_from_either_end() {
        for (a, b) in [
            (Point::new(0.1, 0.2), Point::new(0.7, -0.3)),
            (Point::new(-1.0, 1.0), Point::new(1.0, -1.0)),
            (Point::new(1e-8, 3e7), Point::new(-2.5e-9, 0.0)),
        ] {
            assert_eq!(
                a.midpoint(b).bits(),
                b.midpoint(a).bits(),
                "{a:?} and {b:?} disagree about their midpoint"
            );
        }
    }

    #[test]
    fn a_loop_needs_three_points_to_enclose_anything() {
        let p = Point::new(0.0, 0.0);
        assert!(Contour::new(vec![p, p]).is_none());
        assert_eq!(
            Contour::new(vec![p, p, p]).map(|c| c.points().len()),
            Some(3)
        );
    }
}
