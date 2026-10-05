use crate::number::UnipolarFloat;
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::PI,
    ops::{Add, Mul},
    time::Duration,
};

/// How far a [`Spring`] trails a target that moves at constant speed.
///
/// A lag also sets how quickly the spring lands a jump: half of the jump is
/// covered after about 0.84 × lag and 95 % of it after about 2.37 × lag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lag(Duration);

impl Lag {
    pub const fn from_millis(millis: u64) -> Self {
        Self(Duration::from_millis(millis))
    }

    pub const fn duration(self) -> Duration {
        self.0
    }
}

/// A critically damped spring pulling a position toward a target.
///
/// The spring never overshoots: whatever path the target takes, the position
/// stays within the range the target has covered. Its velocity is continuous,
/// so a target that changes mid-flight bends the motion rather than kinking
/// it. Once the position is within [`Spring::SETTLED`] of a resting target it
/// lands on the target exactly.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Spring {
    position: f64,
    velocity: f64,
}

impl Spring {
    /// Distance from a resting target, in the target's own units, below which
    /// the spring snaps onto the target and stops.
    pub const SETTLED: f64 = 1e-6;

    /// Return a spring resting at the provided position.
    pub const fn at_rest(position: f64) -> Self {
        Self {
            position,
            velocity: 0.0,
        }
    }

    pub const fn position(&self) -> f64 {
        self.position
    }

    /// Return the velocity in position units per second.
    pub const fn velocity(&self) -> f64 {
        self.velocity
    }

    /// Advance the spring by `delta_t` toward a target that holds still for
    /// that interval.
    ///
    /// The step is exact for any `delta_t`: one long step lands where many
    /// short steps covering the same interval would.
    pub fn update(&mut self, target: f64, lag: Lag, delta_t: Duration) {
        let lag_secs = lag.duration().as_secs_f64();
        if lag_secs <= 0.0 {
            *self = Self::at_rest(target);
            return;
        }
        let omega = 2.0 / lag_secs;
        let dt = delta_t.as_secs_f64();
        let offset = self.position - target;
        let drive = self.velocity + offset * omega;
        let decay = (-omega * dt).exp();
        self.position = target + (offset + drive * dt) * decay;
        self.velocity = (self.velocity - drive * omega * dt) * decay;
        if (self.position - target).abs() < Self::SETTLED
            && (self.velocity * lag_secs).abs() < Self::SETTLED
        {
            *self = Self::at_rest(target);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Smooth between two values using a smoothing function.
pub struct Smoother<T: Add<Output = T> + Clone + Copy + Mul<UnipolarFloat, Output = T>> {
    previous: T,
    target: T,
    alpha: UnipolarFloat,
    smooth_time: Duration,
    mode: SmoothMode,
}

impl<T: Add<Output = T> + Clone + Copy + Mul<UnipolarFloat, Output = T>> Smoother<T> {
    pub fn new(initial: T, smooth_time: Duration, mode: SmoothMode) -> Self {
        Self {
            previous: initial,
            target: initial,
            alpha: UnipolarFloat::ONE,
            smooth_time,
            mode,
        }
    }

    /// Set a new target for this smoother.
    pub fn set_target(&mut self, target: T) {
        self.previous = self.val();
        self.target = target;
        self.alpha = UnipolarFloat::ZERO;
    }

    /// Get the current target value.
    pub fn target(&self) -> T {
        self.target
    }

    /// Update the state of this smoother.
    pub fn update_state(&mut self, delta_t: Duration) {
        let delta_alpha = delta_t.as_secs_f64() / self.smooth_time.as_secs_f64();
        self.alpha += delta_alpha;
    }

    /// Return the current smoothed value.
    pub fn val(&self) -> T {
        if self.alpha == UnipolarFloat::ONE {
            return self.target;
        }
        let smoother = match self.mode {
            SmoothMode::Linear => linear,
            SmoothMode::Cosine => cosine,
        };
        let target_weight = smoother(self.alpha);
        (self.target * target_weight) + (self.previous * (UnipolarFloat::ONE - target_weight))
    }
}

#[derive(Copy, Debug, Clone, Serialize, Deserialize)]
pub enum SmoothMode {
    Linear,
    Cosine,
}

// Linear smoothing function.
fn linear(alpha: UnipolarFloat) -> UnipolarFloat {
    alpha
}

// Inverted and scaled cosine smoothing function.
fn cosine(alpha: UnipolarFloat) -> UnipolarFloat {
    let phase = alpha.val() * PI;
    UnipolarFloat::new(-0.5 * phase.cos() + 0.5)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::assert_almost_eq;

    const LAG: Lag = Lag::from_millis(40);
    /// One show tick at 240 fps.
    const TICK: Duration = Duration::from_nanos(4_166_667);
    /// A tick fine enough to locate landmarks in a spring's motion precisely.
    const FINE: Duration = Duration::from_micros(100);

    fn omega() -> f64 {
        2.0 / LAG.duration().as_secs_f64()
    }

    #[test]
    fn spring_jump_matches_the_critically_damped_closed_form() {
        let mut spring = Spring::at_rest(0.0);
        let mut positions = Vec::new();
        for _ in 0..10_000 {
            spring.update(1.0, LAG, FINE);
            positions.push(spring.position());
        }
        for (i, p) in positions.iter().enumerate() {
            let t = FINE.as_secs_f64() * (i + 1) as f64;
            let expected = 1.0 - (1.0 + omega() * t) * (-omega() * t).exp();
            assert!(
                (p - expected).abs() <= Spring::SETTLED,
                "after {t} s: {p} != {expected}"
            );
        }
        let lags_until = |fraction: f64| {
            let i = positions.iter().position(|&p| p >= fraction).unwrap();
            FINE.as_secs_f64() * (i + 1) as f64 / LAG.duration().as_secs_f64()
        };
        assert!((lags_until(0.5) - 0.84).abs() < 0.01, "{}", lags_until(0.5));
        assert!(
            (lags_until(0.95) - 2.37).abs() < 0.01,
            "{}",
            lags_until(0.95)
        );
        assert_eq!(Spring::at_rest(1.0), spring);
    }

    #[test]
    fn spring_trails_a_steadily_moving_target_by_its_lag() {
        let speed = 0.5;
        let mut spring = Spring::at_rest(0.0);
        let mut target = 0.0;
        for _ in 0..20_000 {
            target += speed * FINE.as_secs_f64();
            spring.update(target, LAG, FINE);
        }
        let trailing = Duration::from_secs_f64((target - spring.position()) / speed);
        assert!(
            LAG.duration().abs_diff(trailing) <= FINE,
            "trailed by {trailing:?}"
        );
        // The target advances in small steps, which ripples the velocity by a
        // few parts per million.
        let velocity = spring.velocity();
        assert!((velocity - speed).abs() < speed * 1e-5, "{velocity}");
    }

    #[test]
    fn spring_update_is_exact_for_any_step_length() {
        let mut moving = Spring::at_rest(0.0);
        moving.update(1.0, LAG, Duration::from_millis(10));

        let mut one_step = moving;
        one_step.update(0.3, LAG, Duration::from_millis(60));
        let mut even_steps = moving;
        for _ in 0..60 {
            even_steps.update(0.3, LAG, Duration::from_millis(1));
        }
        let mut uneven_steps = moving;
        for millis in [7, 23, 1, 29] {
            uneven_steps.update(0.3, LAG, Duration::from_millis(millis));
        }
        for other in [even_steps, uneven_steps] {
            assert!((one_step.position() - other.position()).abs() < 1e-12);
            assert!((one_step.velocity() - other.velocity()).abs() < 1e-9);
        }
    }

    #[test]
    fn spring_stays_within_the_range_its_target_has_covered() {
        // The target rises, reverses before the spring arrives, then jumps
        // around faster than the spring can follow.
        let held_targets = [(1.0, 12), (0.3, 30), (0.9, 5), (0.1, 3), (0.6, 120)];
        let mut spring = Spring::at_rest(0.0);
        let (mut low, mut high) = (0.0_f64, 0.0_f64);
        for (target, ticks) in held_targets {
            low = low.min(target);
            high = high.max(target);
            for _ in 0..ticks {
                spring.update(target, LAG, TICK);
                let p = spring.position();
                assert!((low..=high).contains(&p), "{p} outside {low}..={high}");
            }
        }
        assert_eq!(Spring::at_rest(0.6), spring);
    }

    #[test]
    fn spring_with_no_lag_lands_on_its_target_immediately() {
        let mut spring = Spring::at_rest(0.0);
        spring.update(0.7, Lag::from_millis(0), TICK);
        assert_eq!(Spring::at_rest(0.7), spring);
    }

    #[test]
    fn test_cosine_smooth_func() {
        assert_almost_eq(0.0, cosine(UnipolarFloat::ZERO).val());
        assert_almost_eq(1.0, cosine(UnipolarFloat::ONE).val());
        assert_almost_eq(0.5, cosine(UnipolarFloat::new(0.5)).val());
    }

    #[test]
    fn test_smoother() {
        let smooth_time = Duration::from_micros(10);
        let mut smoother: Smoother<f64> = Smoother::new(0.2f64, smooth_time, SmoothMode::Linear);

        assert_almost_eq(0.2, smoother.val());
        smoother.set_target(0.8);
        assert_almost_eq(0.2, smoother.val());

        // Evolve halfway to target.
        smoother.update_state(Duration::from_micros(5));
        assert_almost_eq(0.5, smoother.val());

        // Complete evolution.
        smoother.update_state(Duration::from_micros(5));
        assert_almost_eq(0.8, smoother.val());
    }
}
