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

impl Rgb {
    pub fn from_8bit(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red: UnipolarFloat::new(red as f64 / 127.),
            green: UnipolarFloat::new(green as f64 / 127.),
            blue: UnipolarFloat::new(blue as f64 / 127.),
        }
    }

    pub fn as_hsv(&self) -> Hsv {
        unimplemented!("TODO RGB to HSV conversion.")
    }
}
