//! The coordinate space a figure lives in, and the pieces it is made of.

/// Half the width of the square a figure occupies, and the coordinate of its centre.
pub const CENTER: f64 = 500.0;

/// The width of the square a figure is built around.
///
/// It is not a bound. Nothing here clips, and three families reach outside it:
/// every star lattice does, by up to 860 units, because it is a field cut out of
/// a tiling rather than an object standing in the middle of one, and truchet
/// tilings and phyllotaxes overhang by the half-width of a mark at the border.
/// The figures these were ported from were clipped by the SVG viewBox that
/// carried them, and contours carry no viewBox.
///
/// **Whoever draws a figure has to decide about clipping**, and the decision is
/// not the same for the three: clipping a star lattice is what makes it a field,
/// while clipping a truchet tiling shaves its border marks in half.
pub const EXTENT: f64 = 1000.0;

/// A point in the square from `(0, 0)` to `(EXTENT, EXTENT)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The point at angle `angle` and distance `radius` from the centre of the frame.
    pub fn polar(angle: f64, radius: f64) -> Self {
        Self::new(CENTER + radius * angle.cos(), CENTER + radius * angle.sin())
    }

    /// Distance from the origin.
    pub fn hypot(self) -> f64 {
        self.x.hypot(self.y)
    }
}

/// A closed polyline. The segment from the last point back to the first is implied.
#[derive(Debug, Clone, PartialEq)]
pub struct Contour(pub Vec<Point>);

impl Contour {
    pub fn new(points: Vec<Point>) -> Self {
        Self(points)
    }

    pub fn points(&self) -> &[Point] {
        &self.0
    }

    /// The contour walked backwards.
    pub fn reversed(&self) -> Self {
        let mut points = self.0.clone();
        points.reverse();
        Self(points)
    }
}

impl FromIterator<Point> for Contour {
    fn from_iter<T: IntoIterator<Item = Point>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// How the interior of a figure is decided where its contours overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    /// A region is inside when it is enclosed an odd number of times.
    ///
    /// Overlap subtracts: two contours crossing leave a hole where they meet, and a
    /// contour offset either side of a curve encloses a band rather than a disc.
    EvenOdd,
    /// A region is inside when its winding number is non-zero.
    NonZero,
}

/// A complete figure: closed contours and the rule that decides what they enclose.
///
/// The rule spans every contour, not each one alone, so contours are not
/// independent shapes that happen to be drawn together.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub contours: Vec<Contour>,
    pub fill_rule: FillRule,
}

impl Figure {
    /// A figure whose contours cancel where they overlap.
    pub fn even_odd(contours: Vec<Contour>) -> Self {
        Self {
            contours,
            fill_rule: FillRule::EvenOdd,
        }
    }

    pub fn point_count(&self) -> usize {
        self.contours.iter().map(|c| c.0.len()).sum()
    }
}
