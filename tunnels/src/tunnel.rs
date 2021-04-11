use crate::clock::Clock;
use crate::numbers::{BipolarFloat, UnipolarFloat};
use crate::{
    animation::{self, Animation},
    clock::ClockBank,
};
use serde::{Deserialize, Serialize};
use std::cmp::{max, min};
use std::time::Duration;

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

impl Default for Tunnel {
    fn default() -> Self {
        Self {
            marquee_speed: BipolarFloat(0.0),
            rot_speed: BipolarFloat(0.0),
            thickness: UnipolarFloat(0.0),
            size: UnipolarFloat(0.5),
            aspect_ratio: UnipolarFloat(0.5),
            col_center: UnipolarFloat(0.0),
            col_width: UnipolarFloat(0.0),
            col_spread: UnipolarFloat(0.0),
            col_sat: UnipolarFloat(0.0),
            segs: 126,
            blacking: BipolarFloat(0.15),
            curr_rot_angle: UnipolarFloat(0.0),
            curr_marquee_angle: UnipolarFloat(0.0),
            x_offset: 0.0,
            y_offset: 0.0,
            anims: Default::default(),
        }
    }
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
    pub fn replace_animation(&mut self, anim_num: usize, new_anim: Animation) {
        self.anims[anim_num] = new_anim;
    }

    /// Update the state of this tunnel in preparation for drawing a frame.
    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        // ensure we don't exceed the set bounds of the screen
        self.x_offset = f64::min(f64::max(self.x_offset, -MAX_X_OFFSET), MAX_X_OFFSET);
        self.y_offset = f64::min(f64::max(self.y_offset, -MAX_Y_OFFSET), MAX_Y_OFFSET);

        let mut rot_angle_adjust = 0.0;
        let mut marquee_angle_adjust = 0.0;

        // update the state of the animations and get relevant values
        for anim in &mut self.anims {
            anim.update_state(delta_t);
            // what is this animation targeting?
            // at least for non-chicklet-level targets...
            if let animation::Target::Rotation = anim.target {
                // rotation speed
                rot_angle_adjust += anim.get_value(0., external_clocks) * 0.5;
            } else if let animation::Target::MarqueeRotation = anim.target {
                // marquee rotation speed
                marquee_angle_adjust += anim.get_value(0., external_clocks) * 0.5;
            }
        }

        let timestep_secs = delta_t.as_secs_f64();

        // calulcate the rotation, wrap to 0 to 1
        // delta_t*30. implies the same speed scale as we had at 30fps with evolution tied to frame
        self.curr_rot_angle += UnipolarFloat(
            (scale_speed(self.rot_speed).0 * timestep_secs * 30. + rot_angle_adjust)
                * ROT_SPEED_SCALE,
        );
        self.curr_rot_angle %= 1.;

        // calulcate the marquee angle, wrap to 0 to 1
        // delta_t*30 implies the same speed scale as we had at 30fps with evolution tied to frame
        self.curr_marquee_angle += UnipolarFloat(
            (scale_speed(self.marquee_speed).0 * timestep_secs * 30. + marquee_angle_adjust)
                * ROT_SPEED_SCALE,
        );
        self.curr_marquee_angle %= 1.;
    }
}

/// Scale speeds with a quadratic curve.
/// This provides more resolution for slower speeds.
fn scale_speed(speed: BipolarFloat) -> BipolarFloat {
    let mut scaled = f64::powi(speed.0, 2);
    if speed.0 < 0. {
        scaled *= -1.
    }
    BipolarFloat(scaled)
}

const N_ANIM: usize = 4;
/// legacy tuning parameter; tunnel rotated this many radial units/frame at 30fps
const ROT_SPEED_SCALE: f64 = 0.023;
/// legacy tuning parameter; marquee rotated this many radial units/frame at 30fps
const MARQUEE_SPEED_SCALE: f64 = 0.023;
const BLACKING_SCALE: i32 = 4;
/// max value for size parameter, as a fraction of screen min dimension
const MAX_SIZE: f64 = 1.0;
/// maximum X offset as fraction of screen x-size
const MAX_X_OFFSET: f64 = 0.5;
/// maximum Y offset as fraction of screen y-size
const MAX_Y_OFFSET: f64 = 0.5;
/// X nudge increment as fraction of min half-screen
const X_NUDGE: f64 = 0.025;
/// Y nudge increment as fraction of min half-screen
const Y_NUDGE: f64 = 0.025;
/// line thickness scale as fraction of min half-screen
const THICKNESS_SCALE: f64 = 0.5;
