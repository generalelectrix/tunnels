use crate::number::{BipolarFloat, Phase, UnipolarFloat};
use serde::{Deserialize, Serialize};
use std::{marker::PhantomData, time::Duration};

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

/// A value type that can be smoothed.
///
/// A smoothable value maps onto a continuous axis and back. A type that wraps
/// around, such as [`Phase`], places a value at the point on the axis nearest
/// a given position, so motion along the axis takes the short way round.
pub trait Smoothable: Copy {
    fn to_axis(self) -> f64;

    fn from_axis(position: f64) -> Self;

    /// Return the point on the axis representing this value that lies
    /// nearest to `position`.
    fn axis_near(self, position: f64) -> f64 {
        let _ = position;
        self.to_axis()
    }
}

impl Smoothable for f64 {
    fn to_axis(self) -> f64 {
        self
    }

    fn from_axis(position: f64) -> Self {
        position
    }
}

impl Smoothable for UnipolarFloat {
    fn to_axis(self) -> f64 {
        self.val()
    }

    fn from_axis(position: f64) -> Self {
        Self::new(position)
    }
}

impl Smoothable for BipolarFloat {
    fn to_axis(self) -> f64 {
        self.val()
    }

    fn from_axis(position: f64) -> Self {
        Self::new(position)
    }
}

impl Smoothable for Phase {
    fn to_axis(self) -> f64 {
        self.val()
    }

    fn from_axis(position: f64) -> Self {
        Self::new(position)
    }

    fn axis_near(self, position: f64) -> f64 {
        let mut ahead = (self.val() - position).rem_euclid(1.0);
        if ahead > 0.5 {
            ahead -= 1.0;
        }
        position + ahead
    }
}

/// How quickly a smoothed value follows its target.
pub trait Response {
    const LAG: Lag;
}

/// The response for controls that should feel immediate under the hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fast;

impl Response for Fast {
    const LAG: Lag = Lag::from_millis(40);
}

/// The response for controls whose every change is a large visual move,
/// which should travel rather than jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Slow;

impl Response for Slow {
    const LAG: Lag = Lag::from_millis(100);
}

/// A control value that follows the value asked of it through a [`Spring`].
///
/// The [target](Smoothed::target) is the value most recently asked for: the
/// state to report back to controllers. The [smoothed](Smoothed::smoothed)
/// value is where the spring has got to: the state to render. A smoothed
/// value never leaves the range its targets have covered, takes the same time
/// to land a jump whatever its size, and comes to rest exactly on its target.
/// Its response sets how quickly it follows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Smoothed<T, R = Fast> {
    target: T,
    /// The target's place on the spring's axis.
    target_position: f64,
    spring: Spring,
    #[serde(skip)]
    response: PhantomData<R>,
}

impl<T: Smoothable, R: Response> Smoothed<T, R> {
    /// Return a smoothed value resting at `value`.
    pub fn new(value: T) -> Self {
        Self {
            target: value,
            target_position: value.to_axis(),
            spring: Spring::at_rest(value.to_axis()),
            response: PhantomData,
        }
    }

    /// Return the value most recently asked for.
    pub fn target(&self) -> T {
        self.target
    }

    /// Return the value as it stands on its way to the target.
    pub fn smoothed(&self) -> T {
        T::from_axis(self.spring.position())
    }

    pub fn set_target(&mut self, target: T) {
        self.target = target;
        self.target_position = target.axis_near(self.spring.position());
    }

    /// Advance the smoothed value toward its target by `delta_t`.
    pub fn update_state(&mut self, delta_t: Duration) {
        self.spring.update(self.target_position, R::LAG, delta_t);
        if self.spring == Spring::at_rest(self.target_position) {
            // A wrapping value comes to rest wherever its travels took it on
            // the axis; return it to the target's own place.
            *self = Self::new(self.target);
        }
    }

    /// Bring the smoothed value to rest on its target at once.
    pub fn settle(&mut self) {
        *self = Self::new(self.target);
    }
}

impl<T: Smoothable + Default, R: Response> Default for Smoothed<T, R> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Smoothable, R: Response> From<T> for Smoothed<T, R> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod test {
    use super::*;

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

    /// Run a smoothed value for two seconds of show ticks.
    fn settle<T: Smoothable, R: Response>(value: &mut Smoothed<T, R>) {
        for _ in 0..480 {
            value.update_state(TICK);
        }
    }

    #[test]
    fn smoothed_value_follows_the_target_it_reports() {
        let mut size: Smoothed<UnipolarFloat> = UnipolarFloat::new(0.2).into();
        size.set_target(UnipolarFloat::new(0.8));
        assert_eq!(UnipolarFloat::new(0.8), size.target());
        assert_eq!(UnipolarFloat::new(0.2), size.smoothed());

        size.update_state(TICK);
        let first = size.smoothed().val();
        assert!(first > 0.2 && first < 0.8, "{first}");

        settle(&mut size);
        assert_eq!(UnipolarFloat::new(0.8), size.smoothed());
        assert_eq!(UnipolarFloat::new(0.8), size.target());
    }

    #[test]
    fn a_settled_value_rests_on_its_target() {
        let mut position: Smoothed<f64, Slow> = Smoothed::new(-0.5);
        position.set_target(0.25);
        position.update_state(TICK);
        position.settle();
        assert_eq!(0.25, position.smoothed());
        position.update_state(TICK);
        assert_eq!(0.25, position.smoothed());
    }

    #[test]
    fn each_response_lands_half_a_jump_at_its_own_pace() {
        fn half_a_jump<R: Response>() -> Duration {
            let mut value: Smoothed<f64, R> = Smoothed::new(0.0);
            value.set_target(1.0);
            let mut elapsed = Duration::ZERO;
            for _ in 0..10_000 {
                if value.smoothed() >= 0.5 {
                    break;
                }
                value.update_state(FINE);
                elapsed += FINE;
            }
            elapsed
        }
        for (lag, half) in [
            (Fast::LAG, half_a_jump::<Fast>()),
            (Slow::LAG, half_a_jump::<Slow>()),
        ] {
            let lags = half.as_secs_f64() / lag.duration().as_secs_f64();
            assert!((lags - 0.84).abs() < 0.01, "half a jump after {lags} lags");
        }
        assert!(half_a_jump::<Fast>() < half_a_jump::<Slow>());
    }

    #[test]
    fn smoothed_phase_takes_the_short_way_round() {
        let mut hue: Smoothed<Phase> = Phase::new(0.95).into();
        hue.set_target(Phase::new(0.05));
        for _ in 0..480 {
            hue.update_state(TICK);
            let h = hue.smoothed().val();
            assert!(h >= 0.95 || h <= 0.05, "went the long way through {h}");
        }
        assert_eq!(0.05, hue.smoothed().val());

        // A knob at the top of its range reports back exactly where it was set.
        hue.set_target(UnipolarFloat::ONE.as_phase());
        settle(&mut hue);
        assert_eq!(1.0, hue.target().val());
        assert_eq!(0.0, hue.smoothed().val());
    }
}
