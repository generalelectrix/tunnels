use std::f64::consts::PI;

use crate::numbers::UnipolarFloat;

const TWO_PI: f64 = 2.0 * PI;
const HALF_PI: f64 = PI / 2.0;

pub fn sine(
    mut angle: f64,
    _smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    angle = angle % 1.0;

    if angle > duty_cycle.0 || duty_cycle.0 == 0.0 {
        return 0.0;
    }
    angle = angle / duty_cycle.0;
    if pulse {
        return ((TWO_PI * angle - HALF_PI).sin() + 1.0) / 2.0;
    }
    (TWO_PI * angle).sin()
}

pub fn triangle(
    mut angle: f64,
    _smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    angle = angle % 1.0;

    if angle > duty_cycle.0 || duty_cycle.0 == 0.0 {
        return 0.0;
    }
    angle = angle / duty_cycle.0;
    if pulse {
        return if angle < 0.5 {
            2.0 * angle
        } else {
            2.0 * (1.0 - angle)
        };
    }

    if angle < 0.25 {
        4.0 * angle
    } else if angle > 0.75 {
        4.0 * (angle - 1.0)
    } else {
        2.0 - 4.0 * angle
    }
}

pub fn square(
    mut angle: f64,
    smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    angle = angle % 1.0;

    if angle > duty_cycle.0 || duty_cycle.0 == 0.0 {
        return 0.0;
    }
    angle = angle / duty_cycle.0;
    if pulse {
        return square(angle / 2.0, smoothing, UnipolarFloat(1.0), false);
    }
    if smoothing.0 == 0.0 {
        return if angle < 0.5 { 1.0 } else { -1.0 };
    }

    if angle < smoothing.0 {
        angle / smoothing.0
    } else if angle > (0.5 - smoothing.0) && angle < (0.5 + smoothing.0) {
        -(angle - 0.5) / smoothing.0
    } else if angle > (1.0 - smoothing.0) {
        (angle - 1.0) / smoothing.0
    } else if angle >= smoothing.0 && angle <= 0.5 - smoothing.0 {
        1.0
    } else {
        -1.0
    }
}

pub fn sawtooth(
    mut angle: f64,
    smoothing: UnipolarFloat,
    duty_cycle: UnipolarFloat,
    pulse: bool,
) -> f64 {
    angle = angle % 1.0;

    if angle > duty_cycle.0 || duty_cycle.0 == 0.0 {
        return 0.0;
    }
    angle = angle / duty_cycle.0;
    if pulse {
        return sawtooth(angle / 2.0, smoothing, UnipolarFloat(1.0), false);
    }
    if smoothing.0 == 0.0 {
        return if angle < 0.5 {
            2.0 * angle
        } else {
            2.0 * (angle - 1.0)
        };
    }

    if angle < 0.5 - smoothing.0 {
        angle / (0.5 - smoothing.0)
    } else if angle > 0.5 + smoothing.0 {
        (angle - 1.0) / (0.5 - smoothing.0)
    } else {
        -(angle - 0.5) / smoothing.0
    }
}
