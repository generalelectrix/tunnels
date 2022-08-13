use crate::{
    animation::{Animation, Target},
    clock_bank::ClockBank,
    palette::ColorPalette,
};
use crate::{master_ui::EmitStateChange as EmitShowStateChange, waveforms::sawtooth};
use serde::{Deserialize, Serialize};
use std::cmp::{max, min};
use std::time::Duration;
use tunnels_lib::smooth::{SmoothMode, Smoother};
use tunnels_lib::ArcSegment;
use tunnels_lib::{
    color::Hsv,
    number::{BipolarFloat, Phase, UnipolarFloat},
};
use typed_index_derive::TypedIndex;

#[derive(Clone, Serialize, Deserialize, Debug)]
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
    /// If None: ignore global color palette.
    /// If Some: use this index from the palette to pick the hue.
    /// At present, the saturation and value of the color are ignored.
    palette_selection: Option<usize>,
    /// TODO: regularize segs interface into regular float knobs
    segs: u8,
    /// remove segments at this interval
    ///
    /// bipolar float, internally interpreted as an int on [-16, 16]
    /// defaults to every other chicklet removed
    blacking: BipolarFloat,
    curr_rot_angle: Phase,
    curr_marquee_angle: Phase,
    x_offset: Smoother<f64>,
    y_offset: Smoother<f64>,
    anims: [Animation; N_ANIM],
}

impl Tunnel {
    const MOVE_SMOOTH_TIME: Duration = Duration::from_millis(250);

    pub fn new() -> Self {
        Self {
            marquee_speed: BipolarFloat::ZERO,
            rot_speed: BipolarFloat::ZERO,
            thickness: UnipolarFloat::new(0.1),
            size: UnipolarFloat::new(0.5),
            aspect_ratio: UnipolarFloat::new(0.5),
            col_center: UnipolarFloat::ZERO,
            col_width: UnipolarFloat::ZERO,
            col_spread: UnipolarFloat::ZERO,
            col_sat: UnipolarFloat::ZERO,
            palette_selection: None,
            segs: 126,
            blacking: BipolarFloat::new(0.15),
            curr_rot_angle: Phase::ZERO,
            curr_marquee_angle: Phase::ZERO,
            x_offset: Smoother::new(0.0, Self::MOVE_SMOOTH_TIME, SmoothMode::Linear),
            y_offset: Smoother::new(0.0, Self::MOVE_SMOOTH_TIME, SmoothMode::Linear),
            anims: Default::default(),
        }
    }

    /// Return the blacking parameter, scaled to be an int on [-16, 16].
    ///
    /// If -1, return 1 (-1 implies all segments are black)
    /// If 0, return 1
    fn blacking_integer(&self) -> i32 {
        let scaled = (17. * self.blacking.val()) as i32;
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

    /// Get an iterator over animations.
    pub fn animations(&mut self) -> impl Iterator<Item = &mut Animation> {
        self.anims.iter_mut()
    }

    /// Update the state of this tunnel in preparation for drawing a frame.
    pub fn update_state(&mut self, delta_t: Duration) {
        // ensure we don't exceed the set bounds of the screen
        // self.x_offset = f64::min(f64::max(self.x_offset, -MAX_X_OFFSET), MAX_X_OFFSET);
        // self.y_offset = f64::min(f64::max(self.y_offset, -MAX_Y_OFFSET), MAX_Y_OFFSET);
        // Update smoothers.
        self.x_offset.update_state(delta_t);
        self.y_offset.update_state(delta_t);

        // Update the state of the animations.
        for anim in &mut self.anims {
            anim.update_state(delta_t);
        }
        let timestep_secs = delta_t.as_secs_f64();

        // calulcate the rotation
        // delta_t*30. implies the same speed scale as we had at 30fps with evolution tied to frame
        self.curr_rot_angle +=
            (scale_speed(self.rot_speed).val() * timestep_secs * 30.) * ROT_SPEED_SCALE;

        // calulcate the marquee angle
        // delta_t*30 implies the same speed scale as we had at 30fps with evolution tied to frame
        self.curr_marquee_angle +=
            (scale_speed(self.marquee_speed).val() * timestep_secs * 30.) * MARQUEE_SPEED_SCALE;
    }

    /// Render the current state of the tunnel.
    pub fn render(
        &self,
        level_scale: UnipolarFloat,
        as_mask: bool,
        external_clocks: &ClockBank,
        color_palette: &ColorPalette,
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

            let rel_angle = Phase::new(marquee_interval * seg_num as f64);

            let mut thickness_adjust = 0.;
            let mut size_adjust = 0.;
            let mut aspect_ratio_adjust = 0.;
            let mut col_center_adjust = 0.;
            let mut col_width_adjust = 0.;
            let mut col_period_adjust = 0.;
            let mut col_sat_adjust = 0.;
            let mut x_adjust = 0.;
            let mut y_adjust = 0.;
            let mut rot_angle_adjust = 0.;
            let mut marquee_angle_adjust = 0.;
            // accumulate animation adjustments based on targets
            use Target::*;
            for anim in &self.anims {
                let anim_value = anim.get_value(rel_angle, external_clocks);

                match anim.target {
                    Rotation => rot_angle_adjust += anim_value,
                    MarqueeRotation => marquee_angle_adjust += anim_value,
                    Thickness => thickness_adjust += anim_value,
                    Size => size_adjust += anim_value * 0.5, // limit adjustment
                    AspectRatio => aspect_ratio_adjust += anim_value,
                    Color => col_center_adjust += anim_value * 0.5,
                    ColorSpread => col_width_adjust += anim_value,
                    ColorPeriodicity => col_period_adjust += anim_value * 8.,
                    ColorSaturation => col_sat_adjust += anim_value * 0.5, // limit adjustment
                    PositionX => x_adjust += anim_value,
                    PositionY => y_adjust += anim_value,
                    Blacking => (), // TODO: figure out how this would even make sense
                    Segments => (), // TODO: same here, probably have to lift this animator up to the whole beam level
                }
            }
            // the abs() is there to prevent negative width setting when using multiple animations.
            // TODO: consider if we should change this behavior to make thickness clamp at 0 instead
            // of bounce back via absolute value here.
            let stroke_weight = (self.thickness.val() * (1. + thickness_adjust)).abs();
            let thickness_allowance = self.thickness.val() * THICKNESS_SCALE / 2.;

            // geometry calculations
            let x_center = self.x_offset.val() + x_adjust;
            let y_center = self.y_offset.val() + y_adjust;

            // compute ellipse parameters
            let radius_x = ((self.size.val()
                * (MAX_ASPECT_RATIO * (self.aspect_ratio.val() + aspect_ratio_adjust))
                - thickness_allowance)
                + size_adjust)
                .abs();
            let radius_y = (self.size.val() - thickness_allowance + size_adjust).abs();

            // The angle of this particular segment.
            let start_angle: Phase = self.curr_marquee_angle
                + marquee_interval * (seg_num as f64)
                + marquee_angle_adjust;

            // this angle may exceed 1.0; this is important for correctly displaying
            // arcs that cross the angular origin.
            let stop_angle = start_angle.val() + marquee_interval;

            let rot_angle = self.curr_rot_angle + rot_angle_adjust;

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
                    start: start_angle.val(),
                    stop: stop_angle,
                    rot_angle: rot_angle.val(),
                }
            } else {
                let base_hue = if let Some(palette_index) = self.palette_selection {
                    // TODO: if the palette index is out of range, should we fall
                    // back to something besides zero?
                    color_palette
                        .get(palette_index)
                        .map(|color| color.hue)
                        .unwrap_or(Phase::ZERO)
                        .val()
                } else {
                    self.col_center.val()
                };
                let hue = Phase::new(
                    (base_hue + col_center_adjust)
                        + (0.5
                            * (self.col_width.val() + col_width_adjust)
                            * sawtooth(
                                rel_angle
                                    * ((COLOR_SPREAD_SCALE * self.col_spread.val()).floor()
                                        + col_period_adjust),
                                UnipolarFloat::ZERO,
                                UnipolarFloat::ONE,
                                false,
                            )),
                );

                let sat = UnipolarFloat::new(self.col_sat.val() + col_sat_adjust);

                ArcSegment {
                    level: level_scale.val(),
                    thickness: stroke_weight,
                    hue: hue.val(),
                    sat: sat.val(),
                    val: 1.0,
                    x: x_center,
                    y: y_center,
                    rad_x: radius_x,
                    rad_y: radius_y,
                    start: start_angle.val(),
                    stop: stop_angle,
                    rot_angle: rot_angle.val(),
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
        emitter.emit_tunnel_state_change(PositionX(self.x_offset.target()));
        emitter.emit_tunnel_state_change(PositionY(self.y_offset.target()));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
            NudgeLeft => self.handle_state_change(
                StateChange::PositionX(self.x_offset.target() - X_NUDGE),
                emitter,
            ),
            NudgeRight => self.handle_state_change(
                StateChange::PositionX(self.x_offset.target() + X_NUDGE),
                emitter,
            ),
            NudgeUp => self.handle_state_change(
                StateChange::PositionY(self.y_offset.target() + Y_NUDGE),
                emitter,
            ),
            NudgeDown => self.handle_state_change(
                StateChange::PositionY(self.y_offset.target() - Y_NUDGE),
                emitter,
            ),
            ResetPosition => {
                self.handle_state_change(StateChange::PositionX(0.), emitter);
                self.handle_state_change(StateChange::PositionY(0.), emitter);
            }
            ResetRotation => {
                self.rot_speed = BipolarFloat::ZERO;
                self.curr_rot_angle = Phase::ZERO;
                emitter.emit_tunnel_state_change(StateChange::RotationSpeed(BipolarFloat::ZERO));
            }
            ResetMarquee => {
                self.marquee_speed = BipolarFloat::ZERO;
                self.curr_marquee_angle = Phase::ZERO;
                emitter.emit_tunnel_state_change(StateChange::MarqueeSpeed(BipolarFloat::ZERO));
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
            PaletteSelection(v) => self.palette_selection = v,
            Segments(v) => self.segs = v,
            Blacking(v) => self.blacking = v,
            PositionX(v) => self.x_offset.set_target(v),
            PositionY(v) => self.y_offset.set_target(v),
        };
        emitter.emit_tunnel_state_change(sc);
    }
}

/// Scale speeds with a quadratic curve.
/// This provides more resolution for slower speeds.
fn scale_speed(speed: BipolarFloat) -> BipolarFloat {
    let mut scaled = f64::powi(speed.val(), 2);
    if speed < 0. {
        scaled *= -1.
    }
    BipolarFloat::new(scaled)
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
const COLOR_SPREAD_SCALE: f64 = 16.;
/// X nudge increment
const X_NUDGE: f64 = 0.025;
/// Y nudge increment
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
    PaletteSelection(Option<usize>),
    Segments(u8), // FIXME integer knob
    Blacking(BipolarFloat),
    PositionX(f64),
    PositionY(f64),
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
