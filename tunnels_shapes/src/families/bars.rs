//! Bars, lattices and nested frames: the rectilinear half of the library.

use crate::geom::{Contour, EXTENT, Figure, Point};

/// Which way the bars run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

/// How a run of bars meets the two edges it runs between.
///
/// A run is pitched between two edges rather than laid from one of them, so
/// both edges are treated alike and the figure reads the same either way up.
/// Which of the two treatments a family takes is what its border looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ends {
    /// A bar closes on each edge, so the run fills the frame exactly.
    Closed,
    /// Every bar stands clear of the frame, so the run ends in a gap at each
    /// edge and the outermost marks are whole.
    Clear,
}

/// Where each bar begins and ends along the axis it is pitched across.
fn spans(count: u32, duty: f64, ends: Ends) -> Vec<(f64, f64)> {
    if count == 0 {
        return Vec::new();
    }
    let (pitch, first) = match ends {
        // The last bar closes on the far edge, so what the pitch divides is the
        // frame less that bar's own width: `count - 1` gaps and `duty` of one
        // more pitch.
        Ends::Closed => (EXTENT / (count as f64 - 1.0 + duty), 0.0),
        // Half a gap at each end, which is what centres a whole number of
        // pitches in the frame.
        Ends::Clear => {
            let pitch = EXTENT / count as f64;
            (pitch, pitch * (1.0 - duty) / 2.0)
        }
    };
    (0..count)
        .map(|i| {
            let near = first + i as f64 * pitch;
            (near, near + pitch * duty)
        })
        .collect()
}

/// A run of bars as fillable contours.
fn bars(count: u32, duty: f64, direction: Direction, ends: Ends) -> Vec<Contour> {
    spans(count, duty, ends)
        .into_iter()
        .map(|(near, far)| {
            let corners = match direction {
                Direction::Horizontal => [(0.0, near), (EXTENT, near), (EXTENT, far), (0.0, far)],
                Direction::Vertical => [(near, 0.0), (near, EXTENT), (far, EXTENT), (far, 0.0)],
            };
            corners.into_iter().map(|(x, y)| Point::new(x, y)).collect()
        })
        .collect()
}

/// Parallel bars across the frame, closing on a bar at both ends.
///
/// The run fills the frame exactly, so the pattern is a bar at each edge rather
/// than a bar at one and a gap at the other, and it reads the same turned
/// through half a turn as it does the right way up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slats {
    /// Bars across the frame.
    pub count: u32,
    pub direction: Direction,
    /// What fraction of each pitch the bar fills.
    pub duty: f64,
}

impl Slats {
    pub const fn new(count: u32, direction: Direction, duty: f64) -> Self {
        Self {
            count,
            direction,
            duty,
        }
    }

    pub fn generate(&self) -> Figure {
        Figure::even_odd(bars(self.count, self.duty, self.direction, Ends::Closed))
    }
}

/// Two sets of bars at a right angle, every bar clear of the frame.
///
/// The bars cancel where they cross, so the figure is a lattice of holes rather
/// than a mesh of solid crossings. Holding the run clear of the frame at both
/// ends leaves a whole mark against all four edges, so the border reads as more
/// of the same hatch rather than as a line drawn round it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// Bars in each direction.
    pub count: u32,
    /// What fraction of each pitch a bar fills.
    pub duty: f64,
}

impl Grid {
    pub const fn new(count: u32, duty: f64) -> Self {
        Self { count, duty }
    }

    pub fn generate(&self) -> Figure {
        let mut contours = bars(self.count, self.duty, Direction::Horizontal, Ends::Clear);
        contours.extend(bars(
            self.count,
            self.duty,
            Direction::Vertical,
            Ends::Clear,
        ));
        Figure::even_odd(contours)
    }
}

/// Nested square frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frames {
    /// Frames from the outside in.
    pub steps: u32,
    /// How thick each frame's border is.
    pub thickness: f64,
}

impl Frames {
    pub const fn new(steps: u32, thickness: f64) -> Self {
        Self { steps, thickness }
    }

    pub fn generate(&self) -> Figure {
        if self.steps == 0 {
            return Figure::even_odd(Vec::new());
        }
        let mut contours = Vec::new();
        for i in 0..self.steps {
            let inset = i as f64 * (EXTENT / 2.0 / self.steps as f64);
            let (o, s) = (inset, EXTENT - 2.0 * inset);
            contours.push(square(o, s));
            let (inner_o, inner_s) = (o + self.thickness, s - 2.0 * self.thickness);
            if inner_s > 0.0 {
                contours.push(square_reversed(inner_o, inner_s));
            }
        }
        Figure::even_odd(contours)
    }
}

fn square(origin: f64, size: f64) -> Contour {
    [
        (origin, origin),
        (origin + size, origin),
        (origin + size, origin + size),
        (origin, origin + size),
    ]
    .into_iter()
    .map(|(x, y)| Point::new(x, y))
    .collect()
}

/// The same square walked the other way, so it cuts under a non-zero rule as
/// well as under even-odd.
///
/// The reversal is inert under even-odd, where the square is already a hole by
/// being enclosed twice.
fn square_reversed(origin: f64, size: f64) -> Contour {
    [
        (origin, origin),
        (origin, origin + size),
        (origin + size, origin + size),
        (origin + size, origin),
    ]
    .into_iter()
    .map(|(x, y)| Point::new(x, y))
    .collect()
}
