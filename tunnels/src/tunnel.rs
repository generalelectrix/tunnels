use crate::animation::Animation;
use crate::numbers::{BipolarFloat, UnipolarFloat};
use serde::{Deserialize, Serialize};
use std::cmp::{max, min};

#[derive(Clone, Serialize, Deserialize)]
/// Ellipsoidal tunnels.
///
/// The workhorse.
/// The one and only.
///
/// Ladies and Gentlemen, presenting, The Beating Heart of The Circle Machine.
///
/// TODO: docstring
pub struct Tunnel {
    marquee_speed: BipolarFloat,
    rot_speed: BipolarFloat,
    thickness: UnipolarFloat,
    size: UnipolarFloat,
    aspect_ratio: UnipolarFloat,
    col_center: UnipolarFloat,
    col_width: UnipolarFloat,
    col_spread: UnipolarFloat,
    col_sat: UnipolarFloat,
    /// positive int; could be any number, but previously [0,127]
    ///
    /// TODO: regularize segs interface into regular float knobs
    segs: i32,
    /// remove segments at this interval
    ///
    /// bipolar float, internally interpreted as an int on [-16, 16]
    /// defaults to every other chicklet removed
    blacking: BipolarFloat,
    curr_rot_angle: UnipolarFloat,
    curr_marquee_angle: UnipolarFloat,
    x_offset: f64,
    y_offset: f64,
    anims: [Animation; N_ANIM],
}

impl Tunnel {
    /// Return the blacking parameter, scaled to be an int on [-16, 16].
    ///
    /// If -1, return 1 (-1 implies all segments are black)
    /// If 0, return 1
    fn blacking_integer(&self) -> i32 {
        let scaled = (17. * self.blacking.0) as i32;
        let clamped = max(min(scaled, 16), -16);

        // remote the "all segments blacked" bug
        if clamped >= -1 {
            max(clamped, 1)
        } else {
            clamped
        }
    }

    /// Replace an animation with another.
    fn replace_animation(&mut self, anim_num: usize, new_anim: Animation) {
        self.anims[anim_num] = new_anim;
    }
}

const N_ANIM: usize = 4;

/// tunnel rotates this many radial units/frame at 30fps
const ROT_SPEED_SCALE: f64 = 0.023;

/// marquee rotates this many radial units/frame at 30fps
const MARQUEE_SPEED_SCALE: f64 = 0.023;

const BLACKING_SCALE: i32 = 4;
