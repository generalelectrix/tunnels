use crate::number::UnipolarFloat;
use std::{
    ops::{Add, Mul},
    time::Duration,
};

// Smooth between two values using a smoothing function.
pub struct Smoother<T: Add<Output = T> + Copy + Mul<UnipolarFloat, Output = T>> {
    previous: T,
    target: T,
    alpha: UnipolarFloat,
    smooth_time: Duration,
    f: fn(UnipolarFloat) -> UnipolarFloat,
}

impl<T: Add<Output = T> + Copy + Mul<UnipolarFloat, Output = T>> Smoother<T> {
    pub fn new(initial: T, smooth_time: Duration, f: fn(UnipolarFloat) -> UnipolarFloat) -> Self {
        Self {
            previous: initial,
            target: initial,
            alpha: UnipolarFloat::ONE,
            smooth_time,
            f,
        }
    }

    // Set a new target for this smoother.
    pub fn set_target(&mut self, target: T) {
        self.previous = self.val();
        self.target = target;
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
        let target_weight = (self.f)(self.alpha);
        (self.target * target_weight) + (self.previous * (UnipolarFloat::ONE - target_weight))
    }
}
