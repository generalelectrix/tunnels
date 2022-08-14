use number::{Phase, UnipolarFloat};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::{
    cmp::{max, min},
    hash::{Hash, Hasher},
};

/// A color in the HSV color space.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq)]
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

    pub fn from_hue(hue: f64) -> Self {
        Self {
            hue: Phase::new(hue),
            sat: UnipolarFloat::ONE,
            val: UnipolarFloat::ONE,
        }
    }
}

/// A color in the RGB color space.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq)]
pub struct Rgb {
    pub red: UnipolarFloat,
    pub green: UnipolarFloat,
    pub blue: UnipolarFloat,
}

impl Rgb {
    pub fn from_u8(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red: UnipolarFloat::new(red as f64 / 127.),
            green: UnipolarFloat::new(green as f64 / 127.),
            blue: UnipolarFloat::new(blue as f64 / 127.),
        }
    }

    pub fn from_f32(red: f32, green: f32, blue: f32) -> Self {
        Self {
            red: UnipolarFloat::new(red as f64),
            green: UnipolarFloat::new(green as f64),
            blue: UnipolarFloat::new(blue as f64),
        }
    }

    pub fn as_hsv(&self) -> Hsv {
        let (r, g, b) = (self.red.val(), self.green.val(), self.blue.val());

        let max_val = max(max(OrderedFloat(r), OrderedFloat(g)), OrderedFloat(b)).0;
        let min_val = min(min(OrderedFloat(r), OrderedFloat(g)), OrderedFloat(b)).0;
        let delta = max_val - min_val;

        if delta > 0. {
            Hsv {
                hue: Phase::new(
                    if max_val == r {
                        ((g - b) / delta) % 6.
                    } else if max_val == g {
                        ((b - r) / delta) + 2.
                    } else {
                        ((r - g) / delta) + 4.
                    } / 6.,
                ),
                sat: if max_val > 0. {
                    UnipolarFloat::new(delta / max_val)
                } else {
                    UnipolarFloat::ZERO
                },
                val: UnipolarFloat::new(max_val),
            }
        } else {
            Hsv {
                hue: Phase::ZERO,
                sat: UnipolarFloat::ZERO,
                val: UnipolarFloat::new(max_val),
            }
        }
    }
}

#[test]
fn test_rgb_to_hsv() {
    assert_eq!(
        Rgb::from_u8(255, 255, 255).as_hsv(),
        Hsv {
            hue: Phase::ZERO,
            sat: UnipolarFloat::ZERO,
            val: UnipolarFloat::ONE,
        }
    );
    assert_eq!(
        Rgb::from_u8(0, 0, 0).as_hsv(),
        Hsv {
            hue: Phase::ZERO,
            sat: UnipolarFloat::ZERO,
            val: UnipolarFloat::ZERO,
        }
    );
    assert_eq!(
        Rgb::from_u8(255, 0, 0).as_hsv(),
        Hsv {
            hue: Phase::ZERO,
            sat: UnipolarFloat::ONE,
            val: UnipolarFloat::ONE,
        }
    );
    assert_eq!(
        Rgb::from_u8(0, 255, 0).as_hsv(),
        Hsv {
            hue: Phase::new(1. / 3.),
            sat: UnipolarFloat::ONE,
            val: UnipolarFloat::ONE,
        }
    );
    assert_eq!(
        Rgb::from_u8(0, 0, 255).as_hsv(),
        Hsv {
            hue: Phase::new(2. / 3.),
            sat: UnipolarFloat::ONE,
            val: UnipolarFloat::ONE,
        }
    );
}
