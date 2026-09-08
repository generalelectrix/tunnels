//! Two damped pendulum pairs, one per axis.
//!
//! The frequency pairs are kept apart between the axes: an axis sharing its
//! frequencies with the other traces a line rather than a figure. Each pair is
//! detuned slightly against itself so the figure precesses instead of closing,
//! and nothing in the construction counts, so the family has no arity axis —
//! its density comes from how long the pendulums are followed and how fast they
//! run down.

use crate::curve::{FIT_RADIUS, fit, ribbon, sample};
use crate::geom::Figure;
use std::f64::consts::FRAC_PI_2;

/// One axis: two detuned pendulums, each with its own decay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    pub frequencies: (f64, f64),
    pub damping: (f64, f64),
    /// Phase of the second pendulum, in radians.
    pub phase: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Harmonograph {
    pub x: Axis,
    pub y: Axis,
    /// How long the pendulums are followed.
    pub turns: f64,
    /// Samples taken across that time.
    pub steps: usize,
    /// Width of the ribbon the trace is drawn as.
    pub width: f64,
}

impl Harmonograph {
    pub fn generate(&self) -> Figure {
        let points = sample(
            |t| {
                (
                    (self.x.frequencies.0 * t).sin() * (-self.x.damping.0 * t).exp()
                        + (self.x.frequencies.1 * t + self.x.phase).sin()
                            * (-self.x.damping.1 * t).exp(),
                    (self.y.frequencies.0 * t + FRAC_PI_2).sin() * (-self.y.damping.0 * t).exp()
                        + (self.y.frequencies.1 * t + self.y.phase).sin()
                            * (-self.y.damping.1 * t).exp(),
                )
            },
            0.0,
            self.turns,
            self.steps,
        );
        Figure::even_odd(ribbon(&fit(&points, FIT_RADIUS), self.width, false))
    }
}
