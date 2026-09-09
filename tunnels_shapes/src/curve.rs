//! Turning a bare curve into something a fill rule can act on.
//!
//! A curve encloses no area, so every family built from one gives its curve a
//! width and closes the offsets into contours. A closed curve becomes two
//! contours rather than one: joining the offsets at the seam would lay a spoke
//! of fill across the figure, and on nested rings every spoke lands on the same
//! radius.

use crate::geom::{CENTER, Contour, Point};
use std::f64::consts::{PI, TAU};

/// The radius a sampled curve is scaled to fill.
pub const FIT_RADIUS: f64 = 470.0;

/// The greatest distance a flattened arc may depart from the true arc.
///
/// A figure spans a thousand units and is projected onto an image of roughly a
/// thousand lines, so a tenth of a unit is comfortably under one pixel.
pub const FLATTEN_TOLERANCE: f64 = 0.1;

/// Which way round a circle is walked.
///
/// Direction decides a winding number and nothing else, so under an even-odd
/// fill this makes no difference to what a figure encloses — parity counts
/// crossings, not their sign. It is carried because the figures were built to
/// cut holes under a non-zero rule as well, and because the coordinate sequence
/// is what it is. Do not read a `Negative` circle as evidence that some figure
/// needs a non-zero fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winding {
    /// Increasing angle.
    Positive,
    /// Decreasing angle.
    Negative,
}

/// `count` samples of `f` over `[start, end)`, at even steps of `(end - start) / count`.
///
/// The end of the interval is excluded, so sampling one full period of a closed
/// curve does not repeat its first point.
pub fn sample<F: Fn(f64) -> (f64, f64)>(f: F, start: f64, end: f64, count: usize) -> Vec<Point> {
    (0..count)
        .map(|i| {
            let (x, y) = f(start + (end - start) * i as f64 / count as f64);
            Point::new(x, y)
        })
        .collect()
}

/// Centre a curve on the frame and scale it so its furthest point sits at `radius`.
///
/// Every member of a family then arrives the same size whatever its parameters
/// do to the raw curve's extent.
pub fn fit(points: &[Point], radius: f64) -> Vec<Point> {
    let extent = points.iter().map(|p| p.hypot()).fold(0.0, f64::max);
    let scale = if extent == 0.0 {
        radius
    } else {
        radius / extent
    };
    points
        .iter()
        .map(|p| Point::new(CENTER + p.x * scale, CENTER + p.y * scale))
        .collect()
}

/// Rotate a curve about the centre of the frame.
pub fn turn(points: &[Point], angle: f64) -> Vec<Point> {
    let (sin, cos) = angle.sin_cos();
    points
        .iter()
        .map(|p| {
            let (dx, dy) = (p.x - CENTER, p.y - CENTER);
            Point::new(CENTER + dx * cos - dy * sin, CENTER + dx * sin + dy * cos)
        })
        .collect()
}

/// Give a curve thickness by offsetting it either side into fillable contours.
///
/// A closed curve yields two contours, an outer and an inner, which an even-odd
/// fill resolves into the band between them. An open curve yields one contour,
/// its two offsets joined end to end.
pub fn ribbon(points: &[Point], width: f64, closed: bool) -> Vec<Contour> {
    let n = points.len();
    if n == 0 {
        return Vec::new();
    }
    let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for (i, p) in points.iter().enumerate() {
        let before = if closed || i > 0 {
            points[(i + n - 1) % n]
        } else {
            *p
        };
        let after = if closed || i + 1 < n {
            points[(i + 1) % n]
        } else {
            *p
        };
        let (tx, ty) = (after.x - before.x, after.y - before.y);
        let length = match tx.hypot(ty) {
            0.0 => 1.0,
            len => len,
        };
        let (ox, oy) = (-ty / length * width / 2.0, tx / length * width / 2.0);
        left.push(Point::new(p.x + ox, p.y + oy));
        right.push(Point::new(p.x - ox, p.y - oy));
    }
    if closed {
        right.reverse();
        vec![Contour::new(left), Contour::new(right)]
    } else {
        left.extend(right.into_iter().rev());
        vec![Contour::new(left)]
    }
}

/// A curve of two points given thickness: the straight band joining them.
pub fn chord(a: Point, b: Point, width: f64) -> Vec<Contour> {
    ribbon(&[a, b], width, false)
}

/// The `i`'th of `count` points evenly spaced around a circle centred on the frame.
pub fn on_ring(i: usize, count: usize, radius: f64, phase: f64) -> Point {
    Point::polar(phase + TAU * i as f64 / count as f64, radius)
}

/// `count` points evenly spaced around a circle centred on the frame.
pub fn ring_points(count: usize, radius: f64, phase: f64) -> Vec<Point> {
    (0..count)
        .map(|i| on_ring(i, count, radius, phase))
        .collect()
}

/// A circle as a closed polyline, walked from its leftmost point.
///
/// The number of segments follows from the radius, so the polyline departs from
/// the true circle by no more than [`FLATTEN_TOLERANCE`] whatever its size.
pub fn circle(center: Point, radius: f64, winding: Winding) -> Contour {
    let segments = circle_segments(radius);
    let direction = match winding {
        Winding::Positive => 1.0,
        Winding::Negative => -1.0,
    };
    (0..segments)
        .map(|i| {
            let angle = PI + direction * TAU * i as f64 / segments as f64;
            Point::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
}

/// The fewest segments whose sagitta stays within [`FLATTEN_TOLERANCE`] at this radius.
fn circle_segments(radius: f64) -> usize {
    if radius <= FLATTEN_TOLERANCE {
        return 3;
    }
    let half_angle = (1.0 - FLATTEN_TOLERANCE / radius).acos();
    ((PI / half_angle).ceil() as usize).max(3)
}
