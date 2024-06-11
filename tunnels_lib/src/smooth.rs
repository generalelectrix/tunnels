use crate::number::UnipolarFloat;
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::PI,
    ops::{Add, Mul},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
// Smooth between two values using a smoothing function.
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

    // Set a new target for this smoother.
    pub fn set_target(&mut self, target: T) {
        self.previous = self.val();
        self.target = target;
        self.alpha = UnipolarFloat::ZERO;
    }

    // Get the current target value.
    pub fn target(&self) -> T {
        self.target
    }

    // Update the state of this smoother.
    pub fn update_state(&mut self, delta_t: Duration) {
        let delta_alpha = delta_t.as_secs_f64() / self.smooth_time.as_secs_f64();
        self.alpha += delta_alpha;
    }

    // Return the current smoothed value.
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
