//! Two patterns at slightly different pitch, and the beat between them.
//!
//! Under an even-odd fill the two sets cancel where they cross, and that
//! cancellation is the figure. Too few elements of either and there is no second
//! pattern to beat against.

use crate::curve::ribbon;
use crate::geom::{CENTER, Contour, Figure, Point};
use std::f64::consts::{FRAC_PI_2, PI};

/// The radius the patterns are cut to.
pub const RADIUS: f64 = 470.0;

/// Two ring stacks at slightly different pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoireRings {
    /// Rings in the first stack.
    pub rings: u32,
    /// How many more rings the second stack carries, which sets the beat order.
    pub offset: u32,
    /// Width of the ribbon each ring is drawn as.
    pub width: f64,
}

/// Steps around a ring. The angle advances by two degrees per step.
const RING_STEPS: u32 = 180;

impl MoireRings {
    pub const fn new(rings: u32, offset: u32, width: f64) -> Self {
        Self {
            rings,
            offset,
            width,
        }
    }

    /// Two stacks divide the same radius, so where the counts share a factor a
    /// ring of one lands exactly on a ring of the other and the pair cancels.
    ///
    /// Two curated figures do this: at 16 and 18 rings, and at 22 and 24, one
    /// ring at radius 235 is drawn twice and the even-odd fill erases it. That
    /// is faithful — it is the figure that was judged, and losing one ring out
    /// of forty is why nobody caught it by looking — so it is reproduced rather
    /// than corrected. New figures reached through the arity control are not
    /// allowed to do it; see `arity::ring_offsets`.
    pub fn generate(&self) -> Figure {
        let mut contours = Vec::new();
        for count in [self.rings, self.rings + self.offset] {
            for i in 0..count {
                let radius = RADIUS * (i + 1) as f64 / count as f64;
                let points: Vec<Point> = (0..RING_STEPS)
                    .map(|t| {
                        let angle = PI * t as f64 / 90.0;
                        Point::new(CENTER + radius * angle.cos(), CENTER + radius * angle.sin())
                    })
                    .collect();
                contours.extend(ribbon(&points, self.width, true));
            }
        }
        Figure::even_odd(contours)
    }
}

/// How many directions the bars are laid in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoireKind {
    /// Two sets at a relative angle.
    Grid,
    /// The same again at a right angle, so the beat runs both ways.
    Weave,
}

impl MoireKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Grid => "moire_grid",
            Self::Weave => "moire_weave",
        }
    }
}

/// Bars cut to a disc, laid in sets at a small relative angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoireBars {
    pub kind: MoireKind,
    /// Bars per set.
    pub bars: u32,
    /// Angle between the two sets, in radians.
    pub angle: f64,
}

/// The duty cycle of the bars: half bar, half gap.
const DUTY: f64 = 0.5;

/// A bar shorter than this is dropped rather than drawn as a speck.
const MIN_LENGTH: f64 = 6.0;

impl MoireBars {
    pub const fn new(kind: MoireKind, bars: u32, angle: f64) -> Self {
        Self { kind, bars, angle }
    }

    pub fn generate(&self) -> Figure {
        let half = self.angle / 2.0;
        let angles: &[f64] = match self.kind {
            MoireKind::Grid => &[-half, half],
            MoireKind::Weave => &[-half, half, FRAC_PI_2 - half, FRAC_PI_2 + half],
        };
        let contours = angles
            .iter()
            .flat_map(|&angle| self.disc_bars(angle))
            .collect();
        Figure::even_odd(contours)
    }

    /// One set of parallel bars, clipped to the disc so the figure stays in
    /// frame rather than arriving as a slab.
    fn disc_bars(&self, angle: f64) -> Vec<Contour> {
        if self.bars == 0 {
            return Vec::new();
        }
        let pitch = 2.0 * RADIUS / self.bars as f64;
        let half_width = pitch * DUTY / 2.0;
        let (sin, cos) = angle.sin_cos();
        (0..self.bars)
            .filter_map(|i| {
                let u = -RADIUS + pitch * (i as f64 + 0.5);
                let edge = u.abs() + half_width;
                let length = (RADIUS * RADIUS - edge * edge).max(0.0).sqrt();
                if length < MIN_LENGTH {
                    return None;
                }
                let corners = [
                    (u - half_width, -length),
                    (u + half_width, -length),
                    (u + half_width, length),
                    (u - half_width, length),
                ];
                Some(
                    corners
                        .into_iter()
                        .map(|(x, y)| {
                            Point::new(CENTER + x * cos - y * sin, CENTER + x * sin + y * cos)
                        })
                        .collect::<Contour>(),
                )
            })
            .collect()
    }
}
