//! Bars, lattices and nested frames: the rectilinear half of the library.

use crate::geom::{Contour, EXTENT, Figure, Point};

/// Which way the bars run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

/// Parallel bars across the frame.
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
        Figure::even_odd(self.contours())
    }

    fn contours(&self) -> Vec<Contour> {
        if self.count == 0 {
            return Vec::new();
        }
        let pitch = EXTENT / self.count as f64;
        (0..self.count)
            .map(|i| {
                let near = i as f64 * pitch;
                let far = near + pitch * self.duty;
                let corners = match self.direction {
                    Direction::Horizontal => {
                        [(0.0, near), (EXTENT, near), (EXTENT, far), (0.0, far)]
                    }
                    Direction::Vertical => [(near, 0.0), (near, EXTENT), (far, EXTENT), (far, 0.0)],
                };
                corners.into_iter().map(|(x, y)| Point::new(x, y)).collect()
            })
            .collect()
    }
}

/// Two sets of bars at a right angle.
///
/// The bars cancel where they cross, so the figure is a lattice of holes rather
/// than a mesh of solid crossings.
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
        let mut contours = Slats::new(self.count, Direction::Horizontal, self.duty).contours();
        contours.extend(Slats::new(self.count, Direction::Vertical, self.duty).contours());
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

/// The same square walked the other way, so it cuts rather than fills under a
/// non-zero rule as well as under even-odd.
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
