use number::{Phase, UnipolarFloat};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// A color in the HSV color space.
#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct Hsv {
    pub hue: Phase,
    pub sat: UnipolarFloat,
    pub val: UnipolarFloat,
}

impl Hash for Hsv {
    fn hash<H: Hasher>(&self, state: &mut H) {
        OrderedFloat(self.hue.val()).hash(state);
        OrderedFloat(self.sat.val()).hash(state);
        OrderedFloat(self.val.val()).hash(state);
    }
}

impl Hsv {
    pub const BLACK: Self = Self {
        hue: Phase::ZERO,
        sat: UnipolarFloat::ZERO,
        val: UnipolarFloat::ZERO,
    };
}

/// A color in the RGB color space.
#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct Rgb {
    pub red: UnipolarFloat,
    pub green: UnipolarFloat,
    pub blue: UnipolarFloat,
}
