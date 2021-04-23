use std::{
    cmp::Ordering,
    ops::{Add, AddAssign, Div, Mul, Sub},
};

use derive_more::Display;
use serde::{Deserialize, Serialize};

/// A float type constrained to the range [0.0, 1.0].
/// The type upholds the range invariant by clamping the value to the range.
#[derive(Display, Debug, Copy, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct UnipolarFloat(f64);

impl UnipolarFloat {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    pub fn new(v: f64) -> Self {
        let mut uf = Self(v);
        uf.clamp();
        uf
    }

    pub fn val(&self) -> f64 {
        self.0
    }

    fn clamp(&mut self) {
        clamp(&mut self.0, 0.0, 1.0);
    }
}

impl PartialEq<f64> for UnipolarFloat {
    fn eq(&self, other: &f64) -> bool {
        self.0.eq(other)
    }
}

impl PartialOrd<f64> for UnipolarFloat {
    fn partial_cmp(&self, other: &f64) -> Option<Ordering> {
        self.0.partial_cmp(other)
    }
}

impl Into<f64> for UnipolarFloat {
    fn into(self) -> f64 {
        self.0
    }
}

impl Mul<UnipolarFloat> for UnipolarFloat {
    type Output = UnipolarFloat;
    fn mul(self, rhs: UnipolarFloat) -> Self::Output {
        // This cannot go out of range so no need to clamp.
        Self(self.0 * rhs.0)
    }
}

impl Sub<UnipolarFloat> for UnipolarFloat {
    type Output = UnipolarFloat;
    fn sub(self, rhs: UnipolarFloat) -> Self::Output {
        Self::new(self.0 - rhs.0)
    }
}

impl Add<UnipolarFloat> for UnipolarFloat {
    type Output = UnipolarFloat;
    // Add other to self and clamp.
    fn add(self, rhs: UnipolarFloat) -> Self::Output {
        Self::new(self.val() + rhs.val())
    }
}

impl AddAssign<UnipolarFloat> for UnipolarFloat {
    // Add other to self and clamp.
    fn add_assign(&mut self, rhs: UnipolarFloat) {
        *self += rhs.val();
    }
}

impl AddAssign<f64> for UnipolarFloat {
    // Add other to self and clamp.
    fn add_assign(&mut self, rhs: f64) {
        *self = Self::new(self.0 + rhs);
    }
}

// A float type constrained to the range [-1.0, 1.0].
/// The type upholds the range invariant by clamping the value to the range.
#[derive(Display, Debug, Copy, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BipolarFloat(f64);

impl BipolarFloat {
    pub const ZERO: Self = Self(0.0);

    pub fn new(v: f64) -> Self {
        let mut bf = Self(v);
        bf.clamp();
        bf
    }

    pub fn val(&self) -> f64 {
        self.0
    }

    fn clamp(&mut self) {
        clamp(&mut self.0, -1.0, 1.0);
    }
}

impl PartialEq<f64> for BipolarFloat {
    fn eq(&self, other: &f64) -> bool {
        self.0.eq(other)
    }
}

impl PartialOrd<f64> for BipolarFloat {
    fn partial_cmp(&self, other: &f64) -> Option<Ordering> {
        self.0.partial_cmp(other)
    }
}

impl Into<f64> for BipolarFloat {
    fn into(self) -> f64 {
        self.0
    }
}

impl Mul<UnipolarFloat> for BipolarFloat {
    type Output = BipolarFloat;
    fn mul(self, rhs: UnipolarFloat) -> Self::Output {
        // This cannot go out of range so no need to clamp.
        Self(self.0 * rhs.0)
    }
}

impl Mul<BipolarFloat> for BipolarFloat {
    type Output = BipolarFloat;
    fn mul(self, rhs: BipolarFloat) -> Self::Output {
        // This cannot go out of range so no need to clamp.
        Self(self.0 * rhs.0)
    }
}

fn clamp(v: &mut f64, min: f64, max: f64) {
    *v = f64::min(f64::max(*v, min), max)
}

/// Phase represents a unit angular phase (on the range [0.0, 1.0]).
/// Phase upholds the invariant that the valye contained inside is always in
/// range via wrapping the phase using euclidean modulus.
#[derive(Debug, PartialEq, PartialOrd, Copy, Clone, Serialize, Deserialize)]
pub struct Phase(f64);

impl Phase {
    pub const ZERO: Self = Self(0.0);

    /// Normally this value would always be wrapped back to 0.0, but 1.0 is
    /// an acceptable value for phase and is useful for certain circumstances.
    pub const ONE: Self = Self(1.0);

    pub fn new(v: f64) -> Self {
        let mut p = Self(v);
        p.wrap();
        p
    }

    fn wrap(&mut self) {
        self.0 = self.0.rem_euclid(1.0);
    }

    /// Return the inner phase.
    pub fn val(&self) -> f64 {
        self.0
    }
}

impl Default for Phase {
    fn default() -> Self {
        Self(0.0)
    }
}

impl Add<Phase> for Phase {
    type Output = Phase;
    /// Implement addition as add followed by wrap.
    fn add(self, rhs: Phase) -> Self::Output {
        Self::new(self.0 + rhs.0)
    }
}

impl Add<f64> for Phase {
    type Output = Phase;
    /// Implement addition as add followed by wrap.
    fn add(self, rhs: f64) -> Self::Output {
        Self::new(self.0 + rhs)
    }
}

impl AddAssign<f64> for Phase {
    fn add_assign(&mut self, rhs: f64) {
        *self = *self + rhs;
    }
}

impl Mul<UnipolarFloat> for Phase {
    type Output = Phase;
    fn mul(self, v: UnipolarFloat) -> Self {
        // Can always scale a phase by a unit float as it will never result in
        // an out of range result.
        Self(self.0 * v.val())
    }
}

impl Mul<f64> for Phase {
    type Output = Phase;
    fn mul(self, v: f64) -> Self {
        Self::new(self.0 * v)
    }
}

impl Div<UnipolarFloat> for Phase {
    type Output = Phase;
    /// Divide a phase by a unit float.
    /// The result is wrapped to ensure it is in range.
    fn div(self, v: UnipolarFloat) -> Self {
        Self::new(self.0 / v.val())
    }
}

impl PartialOrd<UnipolarFloat> for Phase {
    fn partial_cmp(&self, other: &UnipolarFloat) -> Option<Ordering> {
        self.0.partial_cmp(&other.val())
    }
}

impl PartialOrd<f64> for Phase {
    fn partial_cmp(&self, other: &f64) -> Option<Ordering> {
        self.0.partial_cmp(other)
    }
}

impl<T: Into<f64> + Copy> PartialEq<T> for Phase {
    fn eq(&self, other: &T) -> bool {
        let o: f64 = (*other).into();
        self.0.eq(&o)
    }
}
