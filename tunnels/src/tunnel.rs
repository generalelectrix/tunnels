use crate::numbers::{BipolarFloat, UnipolarFloat};
use crate::{
    animation::{Animation, Target},
    clock::ClockBank,
};
use crate::{master_ui::EmitStateChange as EmitShowStateChange, waveforms::sawtooth};
use serde::{Deserialize, Serialize};
use std::cmp::{max, min};
use std::time::Duration;
use tunnels_lib::ArcSegment;
use typed_index_derive::TypedIndex;

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
    pub marquee_speed: BipolarFloat,
    pub rot_speed: BipolarFloat,
    pub thickness: UnipolarFloat,
    pub size: UnipolarFloat,
    pub aspect_ratio: UnipolarFloat,
    pub col_center: UnipolarFloat,
    pub col_width: UnipolarFloat,
    pub col_spread: UnipolarFloat,
    pub col_sat: UnipolarFloat,
    /// TODO: regularize segs interface into regular float knobs
    pub segs: u8,
    /// remove segments at this interval
    ///
    /// bipolar float, internally interpreted as an int on [-16, 16]
    /// defaults to every other chicklet removed
    pub blacking: BipolarFloat,
    curr_rot_angle: UnipolarFloat,
    curr_marquee_angle: UnipolarFloat,
    x_offset: f64,
    y_offset: f64,
    anims: [Animation; N_ANIM],
}

impl Tunnel {
    pub fn new() -> Self {
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

    /// Borrow an animation as a mutable reference.
    pub fn animation(&mut self, anim_num: AnimationIdx) -> &mut Animation {
        &mut self.anims[anim_num]
    }

    /// Replace an animation with another.
    pub fn replace_animation(&mut self, anim_num: AnimationIdx, new_anim: Animation) {
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
            if let Target::Rotation = anim.target {
                // rotation speed
                rot_angle_adjust += anim.get_value(0., external_clocks) * 0.5;
            } else if let Target::MarqueeRotation = anim.target {
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

    /// Render the current state of the tunnel.
    pub fn render(
        &self,
        level_scale: UnipolarFloat,
        as_mask: bool,
        external_clocks: &ClockBank,
    ) -> Vec<ArcSegment> {
        // for artistic reasons/convenience, eliminate odd numbers of segments above 40.
        let segs = if self.segs > 40 && self.segs % 2 != 0 {
            self.segs + 1
        } else {
            self.segs
        };
        let blacking = self.blacking_integer();

        let mut arcs = Vec::new();

        let marquee_interval = 1.0 / segs as f64;

        // Iterate over each segment ID and skip the segments that are blacked.
        for seg_num in 0..segs {
            let should_draw_segment = if blacking > 0 {
                (seg_num as i32) % blacking == 0
            } else {
                (seg_num as i32) % blacking != 0
            };
            if !should_draw_segment {
                continue;
            }

            // The angle of this particular segment.
            let start_angle =
                (marquee_interval * (seg_num as f64) + self.curr_marquee_angle.0) % 1.0;
            let rel_angle = marquee_interval * seg_num as f64;

            let mut thickness_adjust = 0.;
            let mut size_adjust = 0.;
            let mut aspect_ratio_adjust = 0.;
            let mut col_center_adjust = 0.;
            let mut col_width_adjust = 0.;
            let mut col_period_adjust = 0.;
            let mut col_sat_adjust = 0.;
            let mut x_adjust = 0.;
            let mut y_adjust = 0.;
            // accumulate animation adjustments based on targets
            use Target::*;
            for anim in &self.anims {
                let anim_value = anim.get_value(rel_angle, external_clocks);

                match anim.target {
                    Thickness => thickness_adjust += anim_value,
                    Size => size_adjust += anim_value * 0.5, // limit adjustment
                    AspectRatio => aspect_ratio_adjust += anim_value,
                    Color => col_center_adjust += anim_value * 0.5,
                    ColorSpread => col_width_adjust += anim_value,
                    ColorPeriodicity => col_period_adjust += anim_value * 8.,
                    ColorSaturation => col_sat_adjust += anim_value * 0.5, // limit adjustment
                    PositionX => x_adjust += anim_value,
                    PositionY => y_adjust += anim_value,
                    _ => (),
                }
            }
            // the abs() is there to prevent negative width setting when using multiple animations.
            // TODO: consider if we should change this behavior to make thickness clamp at 0 instead
            // of bounce back via absolute value here.
            let stroke_weight = (self.thickness.0 * (1. + thickness_adjust)).abs();
            let thickness_allowance = self.thickness.0 * THICKNESS_SCALE / 2.;

            // geometry calculations
            let x_center = self.x_offset + x_adjust;
            let y_center = self.y_offset + y_adjust;

            // this angle may exceed 1.0
            let stop_angle = start_angle + marquee_interval;

            // compute ellipse parameters
            let radius_x = ((self.size.0
                * (MAX_ASPECT_RATIO * (self.aspect_ratio.0 + aspect_ratio_adjust))
                - thickness_allowance)
                + size_adjust)
                .abs();
            let radius_y = (self.size.0 - thickness_allowance + size_adjust).abs();

            let arc = if as_mask {
                ArcSegment {
                    level: 1.0,
                    thickness: stroke_weight,
                    hue: 0.0,
                    sat: 0.0,
                    val: 0.0,
                    x: x_center,
                    y: y_center,
                    rad_x: radius_x,
                    rad_y: radius_y,
                    start: start_angle,
                    stop: stop_angle,
                    rot_angle: self.curr_rot_angle.0,
                }
            } else {
                let mut hue = (self.col_center.0 + col_center_adjust)
                    + (0.5
                        * (self.col_width.0 + col_width_adjust)
                        * sawtooth(
                            rel_angle
                                * ((COLOR_SPREAD_SCALE * self.col_spread.0).floor()
                                    + col_period_adjust),
                            UnipolarFloat(0.0),
                            UnipolarFloat(1.0),
                            false,
                        ));

                hue = hue % 1.0;

                let sat = f64::min(f64::max(self.col_sat.0 + col_sat_adjust, 0.), 1.);

                ArcSegment {
                    level: level_scale.0,
                    thickness: stroke_weight,
                    hue,
                    sat,
                    val: 1.0,
                    x: x_center,
                    y: y_center,
                    rad_x: radius_x,
                    rad_y: radius_y,
                    start: start_angle,
                    stop: stop_angle,
                    rot_angle: self.curr_rot_angle.0,
                }
            };
            arcs.push(arc);
        }
        arcs
    }

    /// Emit the current value of all controllable tunnel state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_tunnel_state_change(MarqueeSpeed(self.marquee_speed));
        emitter.emit_tunnel_state_change(RotationSpeed(self.rot_speed));
        emitter.emit_tunnel_state_change(Thickness(self.thickness));
        emitter.emit_tunnel_state_change(Size(self.size));
        emitter.emit_tunnel_state_change(AspectRatio(self.aspect_ratio));
        emitter.emit_tunnel_state_change(ColorCenter(self.col_center));
        emitter.emit_tunnel_state_change(ColorWidth(self.col_width));
        emitter.emit_tunnel_state_change(ColorSpread(self.col_spread));
        emitter.emit_tunnel_state_change(ColorSaturation(self.col_sat));
        emitter.emit_tunnel_state_change(Segments(self.segs));
        emitter.emit_tunnel_state_change(Blacking(self.blacking));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
            NudgeLeft => self.x_offset -= X_NUDGE,
            NudgeRight => self.x_offset += X_NUDGE,
            NudgeUp => self.y_offset += Y_NUDGE,
            NudgeDown => self.y_offset -= Y_NUDGE,
            ResetPosition => {
                self.x_offset = 0.;
                self.y_offset = 0.;
            }
            ResetRotation => {
                self.rot_speed = BipolarFloat(0.0);
                self.curr_rot_angle = UnipolarFloat(0.0);
                emitter.emit_tunnel_state_change(StateChange::RotationSpeed(BipolarFloat(0.0)));
            }
            ResetMarquee => {
                self.marquee_speed = BipolarFloat(0.0);
                self.curr_marquee_angle = UnipolarFloat(0.0);
                emitter.emit_tunnel_state_change(StateChange::MarqueeSpeed(BipolarFloat(0.0)));
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            MarqueeSpeed(v) => self.marquee_speed = v,
            RotationSpeed(v) => self.rot_speed = v,
            Thickness(v) => self.thickness = v,
            Size(v) => self.size = v,
            AspectRatio(v) => self.aspect_ratio = v,
            ColorCenter(v) => self.col_center = v,
            ColorWidth(v) => self.col_width = v,
            ColorSpread(v) => self.col_spread = v,
            ColorSaturation(v) => self.col_sat = v,
            Segments(v) => self.segs = v,
            Blacking(v) => self.blacking = v,
        };
        emitter.emit_tunnel_state_change(sc);
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

#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(Animation)]
pub struct AnimationIdx(pub usize);

// TODO: move some of these into associated constants
pub const N_ANIM: usize = 4;
/// legacy tuning parameter; tunnel rotated this many radial units/frame at 30fps
const ROT_SPEED_SCALE: f64 = 0.023;
/// legacy tuning parameter; marquee rotated this many radial units/frame at 30fps
const MARQUEE_SPEED_SCALE: f64 = 0.023;
const BLACKING_SCALE: i32 = 4;
const COLOR_SPREAD_SCALE: f64 = 16.;
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
const MAX_ASPECT_RATIO: f64 = 2.0;

pub enum StateChange {
    MarqueeSpeed(BipolarFloat),
    RotationSpeed(BipolarFloat),
    Thickness(UnipolarFloat),
    Size(UnipolarFloat),
    AspectRatio(UnipolarFloat),
    ColorCenter(UnipolarFloat),
    ColorWidth(UnipolarFloat),
    ColorSpread(UnipolarFloat),
    ColorSaturation(UnipolarFloat),
    Segments(u8), // FIXME integer knob
    Blacking(BipolarFloat),
}
pub enum ControlMessage {
    Set(StateChange),
    NudgeLeft,
    NudgeRight,
    NudgeUp,
    NudgeDown,
    ResetPosition,
    ResetRotation,
    ResetMarquee,
}

pub trait EmitStateChange {
    fn emit_tunnel_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_tunnel_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Tunnel(sc))
    }
}
