use crate::animation::{PreparedAnimation, TargetedAnimation};
use crate::layer::{
    ColorAdjust, ColorField, ColorPhase, DrawMode, FigureId, FigureLibrary, FillLayer, GeneratedId,
    Layer, Placement, RenderMode, SegmentLayer, SegmentPath, ShapeGeometry, ShapeMode, SpriteId,
};
use crate::render_context::RenderContext;
use crate::typed_index::typed_index;
use crate::{
    animation_target::AnimationTarget,
    palette::ColorPaletteIdx,
    position_bank::{Position, PositionIdx},
};
use serde::{Deserialize, Serialize};
use std::cmp::max;
use std::time::Duration;
use strum::VariantArray;
use tunnels_lib::number::{BipolarFloat, Phase, UnipolarFloat};
use tunnels_lib::smooth::{SmoothMode, Smoother};
use tunnels_shapes::{Arity, Secondary, ShapeFamily};
use tunnels_sprites::Slot;

/// How often a segment is taken out, on [-16, 16].
///
/// A positive interval keeps every nth segment and a negative one drops every
/// nth, so the two signs are two readings of the same count and a beam runs
/// from mostly dark through solid and out the other side.
///
/// Neither 0 nor -1 is one of these, and for different reasons: -1 takes out
/// every segment, since every index is a multiple of one, and 0 has no meaning
/// at all, because the remainder that decides whether a segment is drawn is not
/// defined against it. Nothing constructs either.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct BlackingInterval(i8);

impl BlackingInterval {
    /// The interval a position of the blacking knob names.
    ///
    /// The knob's two halves are read against different spans because the
    /// detent belongs to the lower one, which is why the travel does not
    /// divide evenly. The bottom of the positive half absorbs the two counts
    /// that are not intervals, so neither reaches a render.
    fn for_knob(knob: u8) -> Self {
        let (knob, centre) = (i32::from(knob), i32::from(KNOB_CENTRE));
        let scaled = if knob <= centre {
            -(17 * (centre - knob) / centre)
        } else {
            17 * (knob - centre) / (i32::from(KNOB_MAX) - centre)
        };
        let clamped = scaled.clamp(-16, 16);
        Self(if clamped >= -1 {
            max(clamped, 1)
        } else {
            clamped
        } as i8)
    }

    /// The knob position that names this interval.
    ///
    /// The middle of the band of positions that name it, so a position
    /// reported to a surface names the interval it came from.
    ///
    /// Found by walking the travel rather than by inverting the arithmetic:
    /// each half of the knob truncates a division, so there is no inverse to
    /// write, and a band is contiguous because neither half turns back on
    /// itself.
    fn knob(self) -> u8 {
        let mut band = (0..=KNOB_MAX).filter(|knob| Self::for_knob(*knob) == self);
        match (band.next(), band.next_back()) {
            (Some(first), Some(last)) => first + (last - first) / 2,
            (Some(only), None) => only,
            // No interval has an empty band, since every one of them came from
            // a position of this knob.
            (None, _) => KNOB_CENTRE,
        }
    }

    /// Whether the segment at this index is drawn.
    fn keeps(self, segment: u8) -> bool {
        let remainder = i32::from(segment) % i32::from(self.0);
        if self.0 > 0 {
            remainder == 0
        } else {
            remainder != 0
        }
    }
}

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
    thickness: Smoother<UnipolarFloat>,
    size: Smoother<UnipolarFloat>,
    aspect_ratio: Smoother<UnipolarFloat>,
    col_center: UnipolarFloat,
    col_width: UnipolarFloat,
    col_spread: UnipolarFloat,
    col_sat: UnipolarFloat,
    /// If None: ignore global color palette.
    /// If Some: use this index from the palette to pick the hue.
    /// At present, the saturation and value of the color are ignored.
    palette_selection: Option<ColorPaletteIdx>,
    /// If None: ignore global position.
    /// If Some: use this index from the positions.
    position_selection: Option<PositionIdx>,
    /// TODO: regularize segs interface into regular float knobs
    segs: u8,
    /// How often a segment is taken out.
    ///
    /// The default takes out every other chicklet.
    blacking: BlackingInterval,
    curr_rot_angle: Phase,
    curr_marquee_angle: Phase,
    spin_speed: BipolarFloat,
    curr_spin_angle: Phase,
    x_offset: Smoother<f64>,
    y_offset: Smoother<f64>,
    anims: [TargetedAnimation; N_ANIM],
    render_mode: RenderMode,
    shape_mode: ShapeMode,
    /// Which coordinate of a figure indexes the color ramp.
    ///
    /// No control surface carries it, so it stays wherever it is set.
    color_phase: ColorPhase,
    /// How much of a figure is painted.
    ///
    /// No control surface carries it, so it stays wherever it is set.
    draw_mode: DrawMode,
    /// Which baked figure a sprite draws.
    ///
    /// Set by the segment and blacking controls, which a figure mode routes
    /// here instead of to `segs` and `blacking`: the first names a family of
    /// the library, the second a figure within it. The whole selection is one
    /// id rather than a pair, so a figure set outright is as good a state as
    /// one arrived at through the knobs, and the position of both knobs falls
    /// back out of the library's shape. Neither is derived from the fields the
    /// other mode reads, so turning a knob in one mode leaves the other alone.
    sprite: SpriteId,
    /// Which figure a generated mode builds.
    ///
    /// The same arrangement as `sprite` over a library that is computed rather
    /// than baked: the whole selection is held resolved, so a figure set
    /// outright is as good a state as one arrived at through the knobs.
    generated: GeneratedId,
}

impl Default for Tunnel {
    fn default() -> Self {
        Self {
            marquee_speed: BipolarFloat::ZERO,
            rot_speed: BipolarFloat::ZERO,
            thickness: Smoother::new(
                UnipolarFloat::new(0.1),
                Self::GEOM_SMOOTH_TIME,
                SmoothMode::Linear,
            ),
            size: Smoother::new(
                UnipolarFloat::new(0.5),
                Self::GEOM_SMOOTH_TIME,
                SmoothMode::Linear,
            ),
            aspect_ratio: Smoother::new(
                UnipolarFloat::new(0.5),
                Self::GEOM_SMOOTH_TIME,
                SmoothMode::Linear,
            ),
            col_center: UnipolarFloat::ZERO,
            col_width: UnipolarFloat::ZERO,
            col_spread: UnipolarFloat::ZERO,
            col_sat: UnipolarFloat::ZERO,
            palette_selection: None,
            position_selection: None,
            segs: 126,
            blacking: BlackingInterval(2),
            curr_rot_angle: Phase::ZERO,
            curr_marquee_angle: Phase::ZERO,
            spin_speed: BipolarFloat::ZERO,
            curr_spin_angle: Phase::ZERO,
            x_offset: Smoother::new(0.0, Self::MOVE_SMOOTH_TIME, SmoothMode::Linear),
            y_offset: Smoother::new(0.0, Self::MOVE_SMOOTH_TIME, SmoothMode::Linear),
            anims: Default::default(),
            render_mode: RenderMode::default(),
            shape_mode: ShapeMode::default(),
            color_phase: ColorPhase::default(),
            draw_mode: DrawMode::default(),
            sprite: SpriteId::default(),
            generated: GeneratedId::default(),
        }
    }
}

impl Tunnel {
    const MOVE_SMOOTH_TIME: Duration = Duration::from_millis(250);
    const GEOM_SMOOTH_TIME: Duration = Duration::from_millis(100);
    /// Where the figure this mode draws sits in its library.
    ///
    /// `None` from a mode that draws segments instead, which has no figure
    /// and no library to place one in.
    fn shelf(&self) -> Option<Shelf> {
        Some(match self.shape_mode.figure_library()? {
            FigureLibrary::Baked => {
                // A figure past the end of the library reads as the first one,
                // which is the same figure it draws.
                let slot = tunnels_sprites::slot(self.sprite.0).unwrap_or(Slot {
                    family: 0,
                    index: 0,
                });
                let families = tunnels_sprites::families();
                Shelf {
                    families: Run {
                        len: families.len() as u16,
                        index: slot.family,
                    },
                    figures: Run {
                        len: families
                            .get(usize::from(slot.family))
                            .map_or(1, |family| family.len),
                        index: slot.index,
                    },
                }
            }
            FigureLibrary::Generated => {
                let run = ArityRun::of(self.generated.family);
                Shelf {
                    families: Run {
                        len: ShapeFamily::ALL.len() as u16,
                        index: generated_family(self.generated.family),
                    },
                    figures: Run {
                        len: run.len,
                        index: run.index_of(self.generated.arity),
                    },
                }
            }
        })
    }

    /// Draw the figure at a place in the open library.
    ///
    /// Which family and how far into it are what is read; how long the family
    /// handed in was is not, because the family named here is the one whose
    /// length now decides. A family shorter than the position asked for gives
    /// up its last figure rather than reaching into the next one, so the two
    /// knobs stay independent: the family knob names a family and nothing else.
    fn select_figure(&mut self, place: Shelf) {
        match self.shape_mode.figure_library() {
            None => (),
            Some(FigureLibrary::Baked) => {
                self.sprite = tunnels_sprites::families()
                    .get(usize::from(place.families.index))
                    .map_or(SpriteId(0), |family| {
                        SpriteId(family.member(place.figures.index))
                    });
            }
            Some(FigureLibrary::Generated) => {
                let family = ShapeFamily::ALL
                    .get(usize::from(place.families.index))
                    .copied()
                    .unwrap_or(ShapeFamily::ALL[0]);
                self.generated.family = family;
                self.generated.arity = ArityRun::of(family).member(place.figures.index);
            }
        }
    }

    /// What the segment control reads, which depends on the mode.
    ///
    /// One control, two state fields: a segment mode's segment count and a figure
    /// mode's family are set by the same knob and stored separately, so
    /// neither is disturbed by work done in the other mode. Changing the mode
    /// therefore moves the knob, because the surface reports state and the
    /// state it is now reporting is a different field. That jump is the
    /// intended behaviour and not a round trip to be stabilized: the two
    /// mappings are unrelated, and the alternative is a surface that lies
    /// about which figure is drawn.
    fn segments_control(&self) -> u8 {
        match self.shelf() {
            None => self.segs - SEGMENTS_MIN,
            Some(place) => KnobTravel::SEGMENTS.position(place.families),
        }
    }

    /// What the blacking control reads, which depends on the mode.
    ///
    /// The second half of the pair the segment control opens: a segment mode
    /// blacks segments out with it, a figure mode picks within the family the
    /// segment control named. Families differ in length, so moving to another
    /// one moves this knob even though the position within the family is
    /// carried across — a surface reporting anything else would name a figure
    /// that is not the one being drawn.
    fn blacking_control(&self) -> u8 {
        match self.shelf() {
            None => self.blacking.knob(),
            Some(place) => KnobTravel::FULL.position(place.figures),
        }
    }

    /// What the marquee control reads, which depends on the mode.
    ///
    /// The third of the controls a figure mode reinterprets: a family reached
    /// by the other two still has a degree of freedom left, and this is the
    /// knob that is otherwise standing at a setting nothing reads.
    fn marquee_control(&self) -> BipolarFloat {
        match self.shape_mode.figure_library() {
            Some(FigureLibrary::Generated) => knob_at(self.generated.secondary),
            _ => self.marquee_speed,
        }
    }

    /// Which render-mode button a surface should light, which depends on the
    /// mode.
    ///
    /// The third of the group the segment control opens: a segment mode picks
    /// how a segment is drawn with these buttons, a figure mode picks how much
    /// of the figure is painted. The two settings are stored apart, so neither
    /// is disturbed by work done in the other mode, and changing the mode
    /// moves the buttons because the state being reported is a different
    /// field.
    ///
    /// The row carries a position and nothing else, and each mode reads that
    /// position against its own list. Neither setting is ever expressed as the
    /// other, which they are not: how a segment is drawn and how much of a
    /// figure is painted have nothing to say about each other.
    fn render_mode_control(&self) -> u8 {
        if self.shape_mode.draws_segments() {
            variant_index(self.render_mode)
        } else {
            variant_index(self.draw_mode)
        }
    }

    /// Borrow an animation as a mutable reference.
    pub fn animation(&mut self, anim_num: AnimationIdx) -> &mut TargetedAnimation {
        &mut self.anims[anim_num]
    }

    /// Replace an animation with another.
    pub fn replace_animation(&mut self, anim_num: AnimationIdx, new_anim: TargetedAnimation) {
        self.anims[anim_num] = new_anim;
    }

    /// Get an iterator over animations.
    pub fn animations(&mut self) -> impl Iterator<Item = &mut TargetedAnimation> {
        self.anims.iter_mut()
    }

    /// Update the state of this tunnel in preparation for drawing a frame.
    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        // Update smoothers.
        self.x_offset.update_state(delta_t);
        self.y_offset.update_state(delta_t);
        self.thickness.update_state(delta_t);
        self.aspect_ratio.update_state(delta_t);
        self.size.update_state(delta_t);

        // Update the state of the animations.
        for anim in &mut self.anims {
            anim.animation.update_state(delta_t, audio_envelope);
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

        // calculate the spin angle
        self.curr_spin_angle +=
            (scale_speed(self.spin_speed).val() * timestep_secs * 30.) * SPIN_SPEED_SCALE;
    }

    /// Render the current state of the tunnel.
    pub fn render(&self, level_scale: UnipolarFloat, as_mask: bool, ctx: RenderContext) -> Layer {
        // Resolve each animation's frame-constant state once. What an animation
        // costs is mostly deciding which clock drives it, where that clock is,
        // where its smoother has got to and what the amplitude works out to —
        // none of which depends on where in the figure the question is asked.
        let anims: [TargetedAnimation<PreparedAnimation>; N_ANIM] =
            std::array::from_fn(|i| self.anims[i].prepare(ctx.clocks, ctx.audio_envelope));

        match self.shape_mode {
            ShapeMode::Ellipse => Layer::Segments(self.render_segments(
                SegmentPath::Ellipse,
                level_scale,
                as_mask,
                ctx,
                &anims,
            )),
            ShapeMode::Line => Layer::Segments(self.render_segments(
                SegmentPath::Line,
                level_scale,
                as_mask,
                ctx,
                &anims,
            )),
            ShapeMode::Sprite => Layer::Fill(self.render_fill(
                FigureId::Baked(self.sprite),
                level_scale,
                as_mask,
                ctx,
                &anims,
            )),
            ShapeMode::Generated => Layer::Fill(self.render_fill(
                FigureId::Generated(self.generated),
                level_scale,
                as_mask,
                ctx,
                &anims,
            )),
        }
    }

    /// The centre of the figure this frame, and the hue its colour starts from.
    ///
    /// Both may be pinned to a bank shared across beams instead of to the
    /// beam's own knobs, so both are resolved the same way for either kind of
    /// layer.
    fn placement_and_hue(&self, ctx: RenderContext) -> (Position, f64) {
        let offset = if let Some(position_idx) = self.position_selection {
            // TODO: if the position index is out of range, should we fall back
            // to something besides zero?
            ctx.positions.get(position_idx).unwrap_or_default()
        } else {
            Position {
                x: self.x_offset.val(),
                y: self.y_offset.val(),
            }
        };

        let base_hue = if let Some(palette_idx) = self.palette_selection {
            // TODO: if the palette index is out of range, should we fall
            // back to something besides zero?
            ctx.palette
                .get(palette_idx)
                .map(|color| color.hue)
                .unwrap_or(Phase::ZERO)
                .val()
        } else {
            self.col_center.val()
        };
        (offset, base_hue)
    }

    /// Render this tunnel as a filled figure.
    ///
    /// A figure is one shape rather than a run of them, so a target that means
    /// the same thing everywhere on it — rotation, thickness, position — is
    /// resolved here into a single number. What is left is the targets that
    /// vary from point to point, which travel with the layer to wherever the
    /// figure's own geometry is.
    fn render_fill(
        &self,
        figure: FigureId,
        level_scale: UnipolarFloat,
        as_mask: bool,
        ctx: RenderContext,
        anims: &[TargetedAnimation<PreparedAnimation>; N_ANIM],
    ) -> FillLayer {
        let (offset, base_hue) = self.placement_and_hue(ctx);

        // A figure has no segment index and no angle around a ring, so a
        // uniform target is asked for its value at the start of its cycle.
        let uniform = |target: AnimationTarget| -> f64 {
            anims
                .iter()
                .filter(|a| a.target == target)
                .map(|a| a.animation.value(Phase::ZERO, 0))
                .sum()
        };

        let placement = Placement {
            x: offset.x + uniform(AnimationTarget::PositionX),
            y: offset.y + uniform(AnimationTarget::PositionY),
            // The tunnel's ellipse formula, onto the figure's two half-extents.
            // A size animation is not folded in here: on a figure it deforms
            // the outline point by point rather than scaling the whole of it.
            extent_x: self.size.val().val() * MAX_ASPECT_RATIO * self.aspect_ratio.val().val(),
            extent_y: self.size.val().val(),
            rot_angle: (self.curr_rot_angle + uniform(AnimationTarget::Rotation)).val(),
        };

        FillLayer {
            figure,
            placement,
            spin_speed: self.spin_speed.val(),
            thickness: (self.thickness.val().val() * (1. + uniform(AnimationTarget::Thickness)))
                .abs(),
            draw_mode: self.draw_mode,
            color: self.color_field(base_hue, level_scale, as_mask),
            color_anims: fill_animations(anims, AnimationTarget::is_color),
            warps: fill_animations(anims, |target| !target.is_color()),
        }
    }

    /// The colour model this beam's shapes are resolved from.
    ///
    /// A mask paints opaque black, punching a hole in whatever lies under it,
    /// so it carries no colour of its own however the colour knobs are set.
    fn color_field(&self, base_hue: f64, level_scale: UnipolarFloat, as_mask: bool) -> ColorField {
        if as_mask {
            ColorField {
                phase: self.color_phase,
                cycles: 0.,
                center: 0.,
                width: 0.,
                sat: 0.,
                val: 0.,
                level: 1.0,
            }
        } else {
            ColorField {
                phase: self.color_phase,
                cycles: (COLOR_SPREAD_SCALE * self.col_spread.val()).floor(),
                center: base_hue,
                width: self.col_width.val(),
                sat: self.col_sat.val(),
                val: 1.0,
                level: level_scale.val(),
            }
        }
    }

    /// Render this tunnel as a run of segments along a path.
    fn render_segments(
        &self,
        segment_path: SegmentPath,
        level_scale: UnipolarFloat,
        as_mask: bool,
        ctx: RenderContext,
        anims: &[TargetedAnimation<PreparedAnimation>; N_ANIM],
    ) -> SegmentLayer {
        // for artistic reasons/convenience, eliminate odd numbers of segments above 40.
        let segs = if self.segs > 40 && !self.segs.is_multiple_of(2) {
            self.segs + 1
        } else {
            self.segs
        };

        let mut arcs = Vec::new();

        let marquee_interval = 1.0 / segs as f64;

        let (offset, base_hue) = self.placement_and_hue(ctx);
        let color_field = self.color_field(base_hue, level_scale, as_mask);
        // A mask paints one colour whatever an animation adds, so it is
        // resolved once here rather than per segment.
        let mask = color_field
            .is_mask()
            .then(|| color_field.sample(Phase::ZERO, ColorAdjust::default()));

        // Iterate over each segment ID and skip the segments that are blacked.
        for seg_num in 0..segs {
            if !self.blacking.keeps(seg_num) {
                continue;
            }

            let rel_angle = Phase::new(marquee_interval * seg_num as f64);

            let mut thickness_adjust = 0.;
            let mut size_adjust = 0.;
            let mut aspect_ratio_adjust = 0.;
            let mut col_center_adjust = 0.;
            let mut col_width_adjust = 0.;
            let mut col_sat_adjust = 0.;
            let mut x_adjust = 0.;
            let mut y_adjust = 0.;
            let mut rot_angle_adjust = 0.;
            let mut marquee_angle_adjust = 0.;
            let mut spin_angle_adjust = 0.;
            // accumulate animation adjustments based on targets
            for anim in anims {
                let anim_value = anim.animation.value(rel_angle, seg_num as usize);

                use AnimationTarget::*;
                match anim.target {
                    Rotation => rot_angle_adjust += anim_value,
                    MarqueeRotation => marquee_angle_adjust += anim_value,
                    Thickness => thickness_adjust += anim_value,
                    Size => size_adjust += anim_value * 0.5, // limit adjustment
                    AspectRatio => aspect_ratio_adjust += anim_value,
                    Color => col_center_adjust += anim_value * 0.5,
                    ColorSpread => col_width_adjust += anim_value,
                    ColorSaturation => col_sat_adjust += anim_value,
                    PositionX => x_adjust += anim_value,
                    PositionY => y_adjust += anim_value,
                    Spin => spin_angle_adjust += anim_value,
                }
            }
            // the abs() is there to prevent negative width setting when using multiple animations.
            // TODO: consider if we should change this behavior to make thickness clamp at 0 instead
            // of bounce back via absolute value here.
            let stroke_weight = (self.thickness.val() * (1. + thickness_adjust)).abs();
            let thickness_allowance = self.thickness.val() * THICKNESS_SCALE / 2.;

            // geometry calculations
            let x_center = offset.x + x_adjust;
            let y_center = offset.y + y_adjust;

            // compute path geometry parameters
            let (extent_x, extent_y) = match segment_path {
                SegmentPath::Ellipse => {
                    let rx = ((self.size.val()
                        * (MAX_ASPECT_RATIO
                            * (self.aspect_ratio.val().val() + aspect_ratio_adjust))
                        - thickness_allowance)
                        + size_adjust)
                        .abs();
                    let ry = (self.size.val().val() - thickness_allowance + size_adjust).abs();
                    (rx, ry)
                }
                SegmentPath::Line => {
                    // size controls line half-length
                    let half_length =
                        (self.size.val().val() - thickness_allowance + size_adjust).abs();
                    // aspect_ratio controls perpendicular offset from line center
                    // at default 0.5, offset is 0 (segments sit on the line)
                    let offset = (self.aspect_ratio.val().val() + aspect_ratio_adjust - 0.5)
                        * MAX_ASPECT_RATIO;
                    (half_length, offset)
                }
            };

            // The angle of this particular segment.
            let start_angle: Phase = self.curr_marquee_angle
                + marquee_interval * (seg_num as f64)
                + marquee_angle_adjust;

            let rot_angle = self.curr_rot_angle + rot_angle_adjust;
            let spin_angle = self.curr_spin_angle + spin_angle_adjust;

            // A segment's index around the path is what indexes the colour
            // ramp, standing in for the coordinate a figure is sampled at.
            let color = mask.unwrap_or_else(|| {
                color_field.sample(
                    rel_angle * color_field.cycles,
                    ColorAdjust {
                        center: col_center_adjust,
                        width: col_width_adjust,
                        sat: col_sat_adjust,
                    },
                )
            });

            arcs.push(ShapeGeometry {
                color,
                placement: Placement {
                    x: x_center,
                    y: y_center,
                    extent_x,
                    extent_y,
                    rot_angle: rot_angle.val(),
                },
                thickness: stroke_weight,
                start: start_angle.val(),
                spin_angle: spin_angle.val(),
            });
        }
        SegmentLayer::new(self.render_mode, segment_path, marquee_interval, arcs)
    }

    /// Emit the current value of all controllable tunnel state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_tunnel_state_change(MarqueeSpeed(self.marquee_control()));
        emitter.emit_tunnel_state_change(RotationSpeed(self.rot_speed));
        emitter.emit_tunnel_state_change(Thickness(self.thickness.target()));
        emitter.emit_tunnel_state_change(Size(self.size.target()));
        emitter.emit_tunnel_state_change(AspectRatio(self.aspect_ratio.target()));
        emitter.emit_tunnel_state_change(ColorCenter(self.col_center));
        emitter.emit_tunnel_state_change(ColorWidth(self.col_width));
        emitter.emit_tunnel_state_change(ColorSpread(self.col_spread));
        emitter.emit_tunnel_state_change(ColorSaturation(self.col_sat));
        emitter.emit_tunnel_state_change(PaletteSelection(self.palette_selection));
        emitter.emit_tunnel_state_change(Segments(self.segments_control()));
        emitter.emit_tunnel_state_change(Blacking(self.blacking_control()));
        emitter.emit_tunnel_state_change(PositionX(self.x_offset.target()));
        emitter.emit_tunnel_state_change(PositionY(self.y_offset.target()));
        emitter.emit_tunnel_state_change(SpinSpeed(self.spin_speed));
        emitter.emit_tunnel_state_change(RenderModeButton(self.render_mode_control()));
        emitter.emit_tunnel_state_change(ColorPhase(self.color_phase));
        emitter.emit_tunnel_state_change(DrawMode(self.draw_mode));
        emitter.emit_tunnel_state_change(ShapeMode(self.shape_mode));
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
            NudgeCW => self.curr_rot_angle += ROT_NUDGE,
            NudgeCCW => self.curr_rot_angle += -ROT_NUDGE,
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
            ResetSpin => {
                self.spin_speed = BipolarFloat::ZERO;
                self.curr_spin_angle = Phase::ZERO;
                emitter.emit_tunnel_state_change(StateChange::SpinSpeed(BipolarFloat::ZERO));
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            // One knob, two fields: a mark mode turns its marquee with it and
            // a generated figure reaches its family's other degree of freedom.
            // A baked figure has neither, and writing the marquee speed from a
            // knob that names nothing would change a mark mode's setting from
            // a mode that has no marquee.
            MarqueeSpeed(v) => match self.shape_mode.figure_library() {
                None => self.marquee_speed = v,
                Some(FigureLibrary::Generated) => self.generated.secondary = secondary_at(v),
                Some(FigureLibrary::Baked) => (),
            },
            RotationSpeed(v) => self.rot_speed = v,
            Thickness(v) => self.thickness.set_target(v),
            Size(v) => self.size.set_target(v),
            AspectRatio(v) => self.aspect_ratio.set_target(v),
            ColorCenter(v) => self.col_center = v,
            ColorWidth(v) => self.col_width = v,
            ColorSpread(v) => self.col_spread = v,
            ColorSaturation(v) => self.col_sat = v,
            PaletteSelection(v) => self.palette_selection = v,
            // One knob, two fields: a segment mode counts segments with it and a
            // figure mode opens a family of the library with it.
            Segments(v) => match self.shelf() {
                None => self.segs = SEGMENTS_MIN + v.min(KNOB_MAX),
                Some(place) => {
                    self.select_figure(Shelf {
                        families: KnobTravel::SEGMENTS.select(v, place.families),
                        ..place
                    });
                    // The position within the family carried across, but the
                    // knob that names it did not: a shorter family puts the
                    // same position somewhere else in the travel.
                    emitter.emit_tunnel_state_change(Blacking(self.blacking_control()));
                }
            },
            // The other half of that pair: a segment mode blacks segments out
            // with it and a figure mode picks within the open family.
            Blacking(v) => match self.shelf() {
                None => self.blacking = BlackingInterval::for_knob(v),
                Some(place) => self.select_figure(Shelf {
                    figures: KnobTravel::FULL.select(v, place.figures),
                    ..place
                }),
            },
            PositionX(v) => self.x_offset.set_target(v),
            PositionY(v) => self.y_offset.set_target(v),
            SpinSpeed(v) => self.spin_speed = v,
            // One row of buttons, two fields: a segment mode picks how a
            // segment is drawn with them and a figure mode picks how much of
            // the figure is painted. A position past the end of the mode's own
            // list names nothing, and changes nothing.
            RenderModeButton(v) => {
                if self.shape_mode.draws_segments() {
                    if let Some(mode) = variant_at(v) {
                        self.render_mode = mode;
                    }
                } else if let Some(mode) = variant_at(v) {
                    self.draw_mode = mode;
                }
            }
            ColorPhase(v) => self.color_phase = v,
            DrawMode(v) => self.draw_mode = v,
            ShapeMode(v) => {
                self.shape_mode = v;
                // Which controls apply depends on the mode, and a surface
                // blanks the ones that do not. Restating the whole tunnel
                // leaves nothing dark that the new mode reads.
                self.emit_state(emitter);
                return;
            }
        };
        emitter.emit_tunnel_state_change(sc);
    }
}

/// The animations driving one half of a figure, resolved for this frame.
///
/// Split in the model rather than in the renderer because the two halves are
/// answered in different places — the colour ones once per ramp texel, the
/// rest once per point of the figure — and neither wants to walk past the
/// other. An animation contributing nothing is dropped rather than asked for a
/// zero tens of thousands of times.
fn fill_animations(
    anims: &[TargetedAnimation<PreparedAnimation>; N_ANIM],
    keep: impl Fn(AnimationTarget) -> bool,
) -> Vec<TargetedAnimation<PreparedAnimation>> {
    anims
        .iter()
        .filter(|a| a.animation.is_active() && a.target.varies_across_figure() && keep(a.target))
        .cloned()
        .collect()
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

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct AnimationIdx(pub usize);
typed_index!(AnimationIdx, TargetedAnimation);

// TODO: move some of these into associated constants
pub const N_ANIM: usize = 4;
/// How many render-mode buttons a surface offers.
///
/// The row drives two settings and carries only a position, so it needs one
/// button per variant of each. A surface offering a different number has a
/// button that names nothing, or a setting no button can reach.
pub const N_RENDER_MODE_BUTTONS: usize = RenderMode::VARIANTS.len();
/// legacy tuning parameter; tunnel rotated this many radial units/frame at 30fps
const ROT_SPEED_SCALE: f64 = 0.023;
/// legacy tuning parameter; marquee rotated this many radial units/frame at 30fps
const MARQUEE_SPEED_SCALE: f64 = 0.023;
/// legacy tuning parameter; spin rotated this many radial units/frame at 30fps
const SPIN_SPEED_SCALE: f64 = 0.023;
const COLOR_SPREAD_SCALE: f64 = 16.;
/// The top of a knob's raw travel, and the position its detent sits at.
///
/// A surface reports a knob in seven bits, so a position is one of 128 and the
/// centre is not the middle of them: the detent is the last position of the
/// lower half, leaving that half one longer than the upper.
pub const KNOB_MAX: u8 = 127;
pub const KNOB_CENTRE: u8 = 64;
/// The segment counts the segment knob's travel covers.
///
/// The knob counts from zero and the count from one, because a beam of no
/// segments is not a beam.
pub const SEGMENTS_MIN: u8 = 1;
pub const SEGMENTS_MAX: u8 = SEGMENTS_MIN + KNOB_MAX;

/// Where a variant sits in its own list of variants.
///
/// A control surface offers one button per variant in that order, so this is
/// also the position of the button that names it.
fn variant_index<T: VariantArray + PartialEq>(value: T) -> u8 {
    T::VARIANTS
        .iter()
        .position(|variant| *variant == value)
        .expect("a variant is in its own list of variants") as u8
}

/// The variant a button position names, or `None` if the row is longer than
/// the list.
fn variant_at<T: VariantArray + Copy>(button: u8) -> Option<T> {
    T::VARIANTS.get(usize::from(button)).copied()
}

/// Where a generated family sits in the order a family control offers them.
fn generated_family(family: ShapeFamily) -> u16 {
    ShapeFamily::ALL
        .iter()
        .position(|f| *f == family)
        .unwrap_or(0) as u16
}

/// The secondary position a knob stands at.
///
/// Read across the whole travel rather than either side of the detent: a
/// family's second degree of freedom has no centre for a detent to mean, which
/// is the same reason the halves of the blacking knob are not distinguished
/// when it names a figure.
fn secondary_at(knob: BipolarFloat) -> Secondary {
    Secondary::new((knob.val() + 1.0) / 2.0)
}

/// Where the knob stands for a secondary position.
fn knob_at(secondary: Secondary) -> BipolarFloat {
    BipolarFloat::new(secondary.get() * 2.0 - 1.0)
}

/// A run of things a knob selects between, and which of them is selected.
///
/// The two travel together because neither answers anything alone: a position
/// means nothing without the length that bounds it, and a length names nothing
/// without the position in it. Passing them as one is also what stops them
/// being handed over in the wrong order, which two bare `u16` arguments
/// invite and no compiler catches.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Run {
    /// How many there are.
    len: u16,
    /// Which of them this is.
    index: u16,
}

/// Where a figure sits in the library it came from, and how much there is to
/// either side of it.
///
/// Both libraries are a list of families and a run of figures in each, so this
/// is what a pair of knobs addresses either of them through: one run for the
/// families and one for the figures of the family that is open.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Shelf {
    families: Run,
    figures: Run,
}

/// The arities of one family, as the run of figures a control walks.
///
/// The generated library's answer to a baked family: a first figure and a
/// length, addressed by how far into it a figure sits. That both libraries
/// answer to the same two questions is what lets one pair of knobs walk
/// either.
#[derive(Copy, Clone, Debug)]
struct ArityRun {
    first: u32,
    len: u16,
}

impl ArityRun {
    fn of(family: ShapeFamily) -> Self {
        let range = family.arity_range();
        Self {
            first: *range.start(),
            len: (range.end() - range.start() + 1) as u16,
        }
    }

    /// The arity at this position in the run, clamped to its last.
    fn member(self, index: u16) -> Arity {
        Arity::new(self.first + u32::from(index.min(self.last())))
    }

    /// How far into the run an arity sits.
    fn index_of(self, arity: Arity) -> u16 {
        arity
            .get()
            .saturating_sub(self.first)
            .min(u32::from(self.last())) as u16
    }

    fn last(self) -> u16 {
        self.len.saturating_sub(1)
    }
}

/// A knob's travel, spread over the positions it selects between.
///
/// Both ends are reachable: the bottom of the travel selects the first
/// position and the top the last. A mapping that indexed the travel directly
/// would leave most of it dead and put the last position out of reach, which
/// is the kind of thing found on stage.
#[derive(Copy, Clone, Debug)]
struct KnobTravel {
    low: u8,
    high: u8,
}

impl KnobTravel {
    /// The whole of a seven-bit knob.
    const FULL: Self = Self {
        low: 0,
        high: KNOB_MAX,
    };

    /// What the segment knob sends, which is the whole of one too: the count
    /// a position names is the mode's business and not the knob's.
    const SEGMENTS: Self = Self::FULL;

    /// The run moved to the position this knob stands at.
    ///
    /// Takes and returns the run rather than a length and an index, so that
    /// the length a position is bounded by cannot arrive from somewhere other
    /// than the run the position lands in. Whichever position the run came in
    /// at is what the knob is replacing.
    ///
    /// Rounded rather than truncated, so the top of the travel reaches the
    /// last position instead of stopping one short.
    fn select(self, knob: u8, run: Run) -> Run {
        let Some(last) = run.len.checked_sub(1).filter(|last| *last > 0) else {
            return Run { index: 0, ..run };
        };
        let span = u32::from(self.high - self.low);
        let knob = u32::from(knob.clamp(self.low, self.high) - self.low);
        let last = u32::from(last);
        Run {
            index: ((2 * knob * last + span) / (2 * span)).min(last) as u16,
            ..run
        }
    }

    /// The knob position that selects where `run` stands in itself.
    ///
    /// Every position has a band of the travel that selects it, and this is
    /// the middle of that band, so a position reported to a surface selects
    /// what it came from when the operator turns the knob back to it.
    fn position(self, run: Run) -> u8 {
        let Some(last) = run.len.checked_sub(1).filter(|last| *last > 0) else {
            return self.middle();
        };
        let span = u32::from(self.high - self.low);
        let (index, last) = (u32::from(run.index.min(last)), u32::from(last));
        self.low + ((2 * index * span + last) / (2 * last)) as u8
    }

    /// The detent: the last position of the lower half of the travel.
    fn middle(self) -> u8 {
        self.low + (self.high - self.low).div_ceil(2)
    }
}
/// X nudge increment
const X_NUDGE: f64 = 0.025;
/// Y nudge increment
const Y_NUDGE: f64 = 0.025;
/// Rotation nudge increment (45° = 1/8 turn)
const ROT_NUDGE: f64 = 0.125;
/// line thickness scale as fraction of min half-screen
const THICKNESS_SCALE: f64 = 0.5;
const MAX_ASPECT_RATIO: f64 = 2.0;

#[derive(Debug)]
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
    PaletteSelection(Option<ColorPaletteIdx>),
    /// Where the segment knob is standing, as the raw position a surface
    /// sends. What it means is the mode's business, not the message's.
    Segments(u8),
    /// Where the blacking knob is standing, as the raw position a surface
    /// sends. What it means is the mode's business, not the message's.
    Blacking(u8),
    PositionX(f64),
    PositionY(f64),
    SpinSpeed(BipolarFloat),
    /// Which of the render-mode buttons is lit, as its position in the row.
    /// What it means is the mode's business, not the message's.
    RenderModeButton(u8),
    ShapeMode(ShapeMode),
    ColorPhase(ColorPhase),
    DrawMode(DrawMode),
}
#[derive(Debug)]
pub enum ControlMessage {
    Set(StateChange),
    NudgeLeft,
    NudgeRight,
    NudgeUp,
    NudgeDown,
    ResetPosition,
    NudgeCW,
    NudgeCCW,
    ResetRotation,
    ResetMarquee,
    ResetSpin,
}

pub trait EmitStateChange {
    fn emit_tunnel_state_change(&mut self, sc: StateChange);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::clock_bank::ClockBank;
    use crate::layer::DrawMode;
    use crate::palette::ColorPalette;
    use crate::position_bank::PositionBank;
    use strum::VariantArray;
    use tunnels_shapes::{Arity, Secondary, ShapeFamily};

    /// An emitter for a test that is about what the tunnel holds rather than
    /// what it reports.
    struct Silent;
    impl EmitStateChange for Silent {
        fn emit_tunnel_state_change(&mut self, _: StateChange) {}
    }

    /// An emitter that keeps what it was told, rendered.
    #[derive(Default)]
    struct Recorder(Vec<String>);
    impl EmitStateChange for Recorder {
        fn emit_tunnel_state_change(&mut self, sc: StateChange) {
            self.0.push(format!("{sc:?}"));
        }
    }

    /// A surface blanks the controls a figure has no use for, so a mode that
    /// does use them has to hear their values again. Restating the whole
    /// tunnel is what leaves nothing dark that the new mode reads.
    #[test]
    fn changing_the_mode_restates_the_controls_a_mode_can_blank() {
        let mut recorder = Recorder::default();
        Tunnel::default()
            .handle_state_change(StateChange::ShapeMode(ShapeMode::Ellipse), &mut recorder);

        let heard = |name: &str| recorder.0.iter().any(|sc| sc.starts_with(name));
        assert!(heard("MarqueeSpeed"), "{:?}", recorder.0);
        assert!(heard("RenderModeButton"), "{:?}", recorder.0);
        assert!(heard("ShapeMode"), "{:?}", recorder.0);
        // The segment and blacking controls read different fields in each
        // mode, so a mode change has to restate them or the surface shows the
        // other mode's values.
        assert!(heard("Segments"), "{:?}", recorder.0);
        assert!(heard("Blacking"), "{:?}", recorder.0);

        // The render-mode buttons are the same story, and what they are
        // restated as is the point: a figure mode reports the button that
        // names its draw mode, not the one the segment mode left lit.
        let mut recorder = Recorder::default();
        Tunnel {
            render_mode: RenderMode::Dot,
            draw_mode: DrawMode::Both,
            ..Default::default()
        }
        .handle_state_change(StateChange::ShapeMode(ShapeMode::Sprite), &mut recorder);
        let names_both = format!("RenderModeButton({})", variant_index(DrawMode::Both));
        assert!(recorder.0.contains(&names_both), "{:?}", recorder.0);
    }

    /// A figure's layer says where to draw a figure and how, and carries no
    /// geometry: what travels is the name of a figure, and the two modes that
    /// fill an area differ only in which library the name comes from.
    #[test]
    fn a_figure_mode_renders_a_placed_figure() {
        let generated = GeneratedId {
            family: ShapeFamily::Rose,
            arity: Arity::new(5),
            secondary: Secondary::new(0.5),
        };
        for (mode, expected) in [
            (ShapeMode::Sprite, FigureId::Baked(SpriteId(7))),
            (ShapeMode::Generated, FigureId::Generated(generated)),
        ] {
            let tunnel = Tunnel {
                shape_mode: mode,
                sprite: SpriteId(7),
                generated,
                ..Default::default()
            };
            let Layer::Fill(fill) = render_fixture(&tunnel) else {
                panic!("{mode:?} renders a figure, not segments");
            };
            assert_eq!(fill.figure, expected);
            // The default half-extents are the ellipse formula's, so a figure
            // and a tunnel at the same knob settings cover the same ground.
            assert_eq!(fill.placement.extent_x, 0.5);
            assert_eq!(fill.placement.extent_y, 0.5);
            assert!(fill.color.is_uniform(), "the default colour is one colour");
        }
    }

    /// The figure controls write the fields their mode reads and leave the
    /// others alone, so work done in one mode survives a trip through another.
    #[test]
    fn the_figure_controls_are_routed_by_the_mode() {
        let mut tunnel = Tunnel::default();
        assert!(
            tunnel.shape_mode.draws_segments(),
            "the default draws segments"
        );
        // The middle of the band of positions naming its interval, so the
        // position a surface is told to stand at is the one it was given.
        let blacking = 88;
        let segments = 36;
        let marquee = BipolarFloat::new(-0.5);

        tunnel.handle_state_change(StateChange::Segments(segments), &mut Silent);
        tunnel.handle_state_change(StateChange::Blacking(blacking), &mut Silent);
        let saucer = variant_index(RenderMode::Saucer);
        tunnel.handle_state_change(StateChange::RenderModeButton(saucer), &mut Silent);
        tunnel.handle_state_change(StateChange::MarqueeSpeed(marquee), &mut Silent);
        assert_eq!(tunnel.segs, segments + SEGMENTS_MIN);
        assert_eq!(tunnel.render_mode, RenderMode::Saucer);
        assert_eq!(
            tunnel.draw_mode,
            DrawMode::default(),
            "the draw mode is untouched"
        );
        assert_eq!(
            tunnel.render_mode_control(),
            saucer,
            "a segment mode reports the render mode"
        );
        assert_eq!(tunnel.blacking, BlackingInterval::for_knob(blacking));
        assert_eq!(tunnel.marquee_speed, marquee);
        assert_eq!(
            tunnel.sprite,
            SpriteId::default(),
            "the figure is untouched"
        );
        assert_eq!(
            tunnel.segments_control(),
            segments,
            "a segment mode reports segs"
        );
        assert_eq!(
            tunnel.generated,
            GeneratedId::default(),
            "the generated figure is untouched"
        );

        // Each figure mode reads the same three knobs into its own library,
        // and neither library disturbs the other or the segment mode's fields.
        for mode in [ShapeMode::Sprite, ShapeMode::Generated] {
            tunnel.handle_state_change(StateChange::ShapeMode(mode), &mut Silent);
            tunnel.handle_state_change(StateChange::Segments(KNOB_MAX), &mut Silent);
            tunnel.handle_state_change(StateChange::Blacking(KNOB_MAX), &mut Silent);
            tunnel.handle_state_change(StateChange::MarqueeSpeed(BipolarFloat::ONE), &mut Silent);

            let place = tunnel.shelf().expect("a figure mode has a shelf");
            assert_eq!(
                (place.families.index, place.figures.index),
                (place.families.len - 1, place.figures.len - 1),
                "{mode:?}: the top of both knobs is the last figure of the last family"
            );
            assert_eq!(
                tunnel.segments_control(),
                KnobTravel::SEGMENTS.position(place.families),
                "{mode:?} reports the family"
            );
            assert_eq!(
                tunnel.blacking_control(),
                KnobTravel::FULL.position(place.figures),
                "{mode:?} reports the position within it"
            );

            assert_eq!(
                tunnel.segs,
                segments + SEGMENTS_MIN,
                "{mode:?}: the segment count is untouched"
            );
            assert_eq!(
                tunnel.blacking,
                BlackingInterval::for_knob(blacking),
                "{mode:?}: the blacking is untouched"
            );
        }

        // Only a generated figure reads the marquee knob; a baked one leaves
        // the marquee speed where a segment mode had it.
        assert_eq!(
            tunnel.marquee_speed, marquee,
            "the marquee speed was not written by a figure mode"
        );
        assert_eq!(
            tunnel.generated.secondary,
            Secondary::new(1.0),
            "the top of the marquee knob is the top of the secondary travel"
        );
        assert_eq!(tunnel.marquee_control(), BipolarFloat::ONE);

        // The same three buttons, naming how much of a figure is painted.
        let outline = variant_index(DrawMode::Outline);
        tunnel.handle_state_change(StateChange::RenderModeButton(outline), &mut Silent);
        assert_eq!(tunnel.draw_mode, DrawMode::Outline);
        assert_eq!(
            tunnel.render_mode,
            RenderMode::Saucer,
            "the render mode is untouched"
        );
        assert_eq!(
            tunnel.render_mode_control(),
            outline,
            "a figure mode reports the draw mode as the button that names it"
        );

        tunnel.handle_state_change(StateChange::ShapeMode(ShapeMode::Ellipse), &mut Silent);
        assert_eq!(
            tunnel.segments_control(),
            segments,
            "the segment count came back unchanged"
        );
        assert_eq!(
            tunnel.blacking_control(),
            blacking,
            "the blacking came back unchanged"
        );
        assert_eq!(
            tunnel.render_mode_control(),
            saucer,
            "the render mode came back unchanged"
        );
        assert_eq!(
            tunnel.marquee_control(),
            marquee,
            "the marquee speed came back unchanged"
        );
    }

    /// One row of buttons drives two lists, so the lists have to be the same
    /// length and a position has to mean the same thing going in as coming
    /// back out.
    ///
    /// Nothing converts between the two settings any more, so no exhaustive
    /// match fails to build when a variant is added to one list and not the
    /// other. This is what catches it instead: the row would have a position
    /// that one mode reads and the other ignores.
    #[test]
    fn a_button_position_means_the_same_thing_to_both_modes() {
        assert_eq!(
            RenderMode::VARIANTS.len(),
            DrawMode::VARIANTS.len(),
            "one row of buttons drives both"
        );

        for button in 0..RenderMode::VARIANTS.len() as u8 {
            for (mode, reads_render) in [(ShapeMode::Ellipse, true), (ShapeMode::Sprite, false)] {
                let mut tunnel = Tunnel {
                    shape_mode: mode,
                    ..Default::default()
                };
                tunnel.handle_state_change(StateChange::RenderModeButton(button), &mut Silent);
                assert_eq!(
                    tunnel.render_mode_control(),
                    button,
                    "button {button} came back as another position in {mode:?}"
                );
                if reads_render {
                    assert_eq!(
                        tunnel.render_mode,
                        RenderMode::VARIANTS[usize::from(button)]
                    );
                } else {
                    assert_eq!(tunnel.draw_mode, DrawMode::VARIANTS[usize::from(button)]);
                }
            }
        }
    }

    /// The knob positions reported for a figure select that figure, so an
    /// operator who turns the knobs back to where the surface put them gets
    /// the figure the surface named.
    #[test]
    fn the_reported_knob_positions_select_the_figure_they_name() {
        for id in 0..tunnels_sprites::count() as u16 {
            let mut tunnel = Tunnel {
                shape_mode: ShapeMode::Sprite,
                sprite: SpriteId(id),
                ..Default::default()
            };
            let segs = tunnel.segments_control();
            let blacking = tunnel.blacking_control();
            assert!(segs <= KNOB_MAX, "figure {id}");

            tunnel.sprite = SpriteId(0);
            tunnel.handle_state_change(StateChange::Segments(segs), &mut Silent);
            tunnel.handle_state_change(StateChange::Blacking(blacking), &mut Silent);
            assert_eq!(
                tunnel.sprite,
                SpriteId(id),
                "the positions reported for figure {id} select another figure"
            );
        }
    }

    /// Both knobs' travel covers what they choose between and reaches both
    /// ends, because a knob that cannot get to the last one is found on stage.
    ///
    /// A knob sends 128 positions, so it reaches every one of a shorter list
    /// and steps evenly through a longer one. Every family of figures is
    /// shorter; an arity range can be longer, and a count is a quantity rather
    /// than a list, so stepping through one loses nothing a list would lose.
    #[test]
    fn the_figure_knobs_reach_every_family_and_every_figure_in_one() {
        /// Every position a knob's travel selects, in the order it selects
        /// them, checked for reaching both ends without ever going back.
        fn reached(travel: KnobTravel, count: u16, what: &str) -> Vec<u16> {
            let mut reached: Vec<u16> = (travel.low..=travel.high)
                .map(|knob| {
                    travel
                        .select(
                            knob,
                            Run {
                                len: count,
                                index: 0,
                            },
                        )
                        .index
                })
                .collect();
            assert_eq!(reached.first().copied(), Some(0), "the floor of {what}");
            assert_eq!(
                reached.last().copied(),
                Some(count - 1),
                "the ceiling of {what}"
            );
            assert!(
                reached.windows(2).all(|w| w[0] <= w[1]),
                "turning the knob up went back in {what}"
            );
            reached.dedup();
            let positions = u16::from(travel.high - travel.low) + 1;
            assert_eq!(
                reached.len(),
                usize::from(count.min(positions)),
                "the knob selects {} of the {count} positions of {what}",
                reached.len()
            );
            reached
        }

        for (library, families) in [
            (
                "the baked library",
                tunnels_sprites::families().len() as u16,
            ),
            ("the generated library", ShapeFamily::ALL.len() as u16),
        ] {
            reached(KnobTravel::SEGMENTS, families, library);
        }

        let baked = tunnels_sprites::families()
            .iter()
            .map(|family| (family.name.to_string(), family.len));
        let generated = ShapeFamily::ALL.iter().map(|family| {
            let range = family.arity_range();
            (
                family.name().to_string(),
                (range.end() - range.start() + 1) as u16,
            )
        });
        for (name, len) in baked.chain(generated) {
            reached(KnobTravel::FULL, len, &format!("the {name} family"));
        }
    }

    /// Every knob position gives the interval it names, over the whole travel,
    /// and the position reported back for that interval names it again.
    ///
    /// The interval is a ratio of the knob's position to the span its half of
    /// the travel covers, truncated. Stated here as that ratio, in floating
    /// point, against the integer arithmetic that computes it — the two agree
    /// for all 128 positions, and an interval is a count of segments, so a
    /// position that landed a step either side of the ratio would black the
    /// wrong ones.
    ///
    /// The round trip is what a surface sees: an interval is reported as a
    /// position, and an operator who leaves that position alone must not have
    /// the beam change under them.
    #[test]
    fn the_blacking_interval_is_the_ratio_the_knob_stands_at() {
        for knob in 0..=KNOB_MAX {
            let centre = f64::from(KNOB_CENTRE);
            let span = if knob <= KNOB_CENTRE {
                centre
            } else {
                f64::from(KNOB_MAX) - centre
            };
            let ratio = (17.0 * (f64::from(knob) - centre) / span) as i32;
            let clamped = ratio.clamp(-16, 16);
            let expected = if clamped >= -1 {
                max(clamped, 1)
            } else {
                clamped
            };

            let interval = BlackingInterval::for_knob(knob);
            assert_eq!(
                interval,
                BlackingInterval(expected as i8),
                "knob position {knob} names the wrong interval"
            );
            assert_eq!(
                BlackingInterval::for_knob(interval.knob()),
                interval,
                "the position reported for knob {knob} names another interval"
            );
        }
    }

    /// No knob position gives an interval of 0 or -1: the first is the divisor
    /// of a remainder and cannot be zero, and the second takes out every
    /// segment, leaving a beam indistinguishable from a broken one.
    #[test]
    fn no_knob_position_leaves_nothing_to_look_at() {
        for knob in 0..=KNOB_MAX {
            let interval = BlackingInterval::for_knob(knob).0;
            assert!(
                interval >= 1 || interval <= -2,
                "knob position {knob} gives an interval of {interval}"
            );
        }
    }

    /// Opening another family keeps the position within it, and reports the
    /// knob position that names where the figure now sits.
    #[test]
    fn changing_family_carries_the_position_within_it() {
        let mut tunnel = Tunnel {
            shape_mode: ShapeMode::Sprite,
            ..Default::default()
        };
        tunnel.handle_state_change(StateChange::Blacking(KNOB_CENTRE), &mut Silent);
        let index = tunnel
            .shelf()
            .expect("a figure mode has a shelf")
            .figures
            .index;
        assert!(index > 0, "the middle of a family is not its first figure");

        let mut recorder = Recorder::default();
        tunnel.handle_state_change(StateChange::Segments(KNOB_MAX), &mut recorder);
        let place = tunnel.shelf().expect("a figure mode has a shelf");
        assert_eq!(place.families.index, place.families.len - 1);
        assert_eq!(
            place.figures.index,
            index.min(place.figures.len - 1),
            "the position within the family did not carry across"
        );
        assert!(
            recorder.0.iter().any(|sc| sc.starts_with("Blacking")),
            "the surface was not told where the selection knob now sits: {:?}",
            recorder.0
        );
    }

    /// A masked figure paints opaque black, punching a hole in what is under
    /// it — the same value a masked segment carries.
    #[test]
    fn a_masked_figure_is_opaque_black() {
        let tunnel = Tunnel {
            shape_mode: ShapeMode::Sprite,
            col_width: UnipolarFloat::ONE,
            col_spread: UnipolarFloat::ONE,
            ..Default::default()
        };
        let Layer::Fill(fill) = tunnel.render(
            UnipolarFloat::ONE,
            true,
            RenderContext {
                clocks: &ClockBank::default().as_static(),
                palette: &ColorPalette::default(),
                positions: &PositionBank::default(),
                audio_envelope: UnipolarFloat::ZERO,
            },
        ) else {
            panic!("a sprite renders a figure, not segments");
        };
        assert_eq!(fill.color.val, 0.0);
        assert_eq!(fill.color.level, 1.0);
        assert!(
            fill.color.is_uniform(),
            "a mask is one colour however the colour knobs are set"
        );
    }

    fn render_fixture(tunnel: &Tunnel) -> Layer {
        tunnel.render(
            UnipolarFloat::ONE,
            false,
            RenderContext {
                clocks: &ClockBank::default().as_static(),
                palette: &ColorPalette::default(),
                positions: &PositionBank::default(),
                audio_envelope: UnipolarFloat::ZERO,
            },
        )
    }
}

pub mod fixture {
    use std::time::Duration;

    use crate::layer::{ColorPhase, DrawMode, Layer, LayerCollection, RenderMode, ShapeMode};
    use tunnels_lib::number::{BipolarFloat, UnipolarFloat};

    use crate::animation::{
        ControlMessage as AnimControlMessage, StateChange as AnimStateChange, Waveform,
    };
    use crate::animation_target::AnimationTarget;
    use crate::clock_bank::{ClockBank, ClockIdx};
    use crate::palette::ColorPalette;
    use crate::position_bank::PositionBank;
    use strum::VariantArray;

    use super::*;

    struct NoopEmitter;

    impl EmitStateChange for NoopEmitter {
        fn emit_tunnel_state_change(&mut self, _: StateChange) {}
    }

    impl crate::animation::EmitStateChange for NoopEmitter {
        fn emit_animation_state_change(&mut self, _: crate::animation::StateChange) {}
    }

    fn render_default(tunnel: &Tunnel) -> Layer {
        tunnel.render(
            UnipolarFloat::ONE,
            false,
            RenderContext {
                clocks: &ClockBank::default().as_static(),

                palette: &ColorPalette::default(),

                positions: &PositionBank::default(),

                audio_envelope: UnipolarFloat::ZERO,
            },
        )
    }

    fn snapshot(layer: Layer) -> LayerCollection {
        vec![layer]
    }

    /// Configure a tunnel for stress testing.
    ///
    /// `marquee_speed` is parameterized because the stress test varies it
    /// across channels: `-1.0 + (2.0 * i / channel_count)`.
    pub fn configure_stress(tunnel: &mut Tunnel, marquee_speed: BipolarFloat) {
        tunnel.handle_state_change(
            StateChange::ColorWidth(UnipolarFloat::new(0.25)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorSpread(UnipolarFloat::ONE),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorSaturation(UnipolarFloat::new(0.25)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(StateChange::MarqueeSpeed(marquee_speed), &mut NoopEmitter);
        tunnel.handle_state_change(StateChange::Blacking(KNOB_CENTRE), &mut NoopEmitter);

        for (i, anim) in tunnel.anims.iter_mut().enumerate() {
            anim.animation.control(
                AnimControlMessage::Set(AnimStateChange::Waveform(match i % 4 {
                    0 => Waveform::Sine,
                    1 => Waveform::Triangle,
                    2 => Waveform::Square,
                    _ => Waveform::Sawtooth,
                })),
                &mut NoopEmitter,
            );
            anim.animation.control(
                AnimControlMessage::Set(AnimStateChange::Speed(BipolarFloat::new(i as f64 / 3.0))),
                &mut NoopEmitter,
            );
            anim.animation.control(
                AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::new(0.5))),
                &mut NoopEmitter,
            );
            anim.target = AnimationTarget::Thickness;
            anim.animation.control(
                AnimControlMessage::Set(AnimStateChange::NPeriods(3)),
                &mut NoopEmitter,
            );
        }
    }

    /// Render a default tunnel to a snapshot for use in test fixtures.
    pub fn default_tunnel_snapshot() -> LayerCollection {
        snapshot(render_default(&Tunnel::default()))
    }

    /// Render a tunnel with aspect ratio set halfway towards max for elliptical shape.
    pub fn elliptical_tunnel_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel::default();
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.75)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a stress-configured tunnel to a snapshot for use in test fixtures.
    pub fn stress_tunnel_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel::default();
        configure_stress(&mut tunnel, BipolarFloat::new(-1.0));
        snapshot(render_default(&tunnel))
    }

    /// Render a default tunnel in dot mode for snapshot testing.
    pub fn default_tunnel_dot_snapshot() -> LayerCollection {
        let tunnel = Tunnel {
            render_mode: RenderMode::Dot,
            ..Default::default()
        };
        snapshot(render_default(&tunnel))
    }

    /// Render a stress-configured tunnel in dot mode for snapshot testing.
    pub fn stress_tunnel_dot_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Dot,
            ..Default::default()
        };
        configure_stress(&mut tunnel, BipolarFloat::new(-1.0));
        snapshot(render_default(&tunnel))
    }

    /// Render an elliptical tunnel in dot mode for snapshot testing.
    pub fn elliptical_tunnel_dot_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Dot,
            ..Default::default()
        };
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.75)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Set how many segments a beam draws, the way a control surface would.
    ///
    /// The knob counts from zero and a segment count from one, so a fixture
    /// states the count it wants drawn and this finds the position for it.
    fn set_segments(tunnel: &mut Tunnel, segments: u8) {
        tunnel.handle_state_change(
            StateChange::Segments(segments - SEGMENTS_MIN),
            &mut NoopEmitter,
        );
    }

    fn saucer_tunnel(segs: u8, thickness: f64) -> Tunnel {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Saucer,
            ..Default::default()
        };
        set_segments(&mut tunnel, segs);
        tunnel.handle_state_change(
            StateChange::Thickness(UnipolarFloat::new(thickness)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        tunnel
    }

    /// Render a saucer tunnel with few thin segments for snapshot testing.
    pub fn saucer_few_thin_snapshot() -> LayerCollection {
        snapshot(render_default(&saucer_tunnel(12, 0.1)))
    }

    /// Render a saucer tunnel with many thick segments for snapshot testing.
    pub fn saucer_many_thick_snapshot() -> LayerCollection {
        snapshot(render_default(&saucer_tunnel(126, 0.5)))
    }

    /// Render a saucer tunnel on a wide ellipse for snapshot testing.
    pub fn saucer_wide_ellipse_snapshot() -> LayerCollection {
        let mut tunnel = saucer_tunnel(12, 0.1);
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.75)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a saucer tunnel on a tall ellipse for snapshot testing.
    pub fn saucer_tall_ellipse_snapshot() -> LayerCollection {
        let mut tunnel = saucer_tunnel(12, 0.1);
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.25)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Configure a saucer tunnel with spin animation on the first animator.
    /// Sets target to Spin with full amplitude (size=1) using the default sine waveform.
    fn saucer_spin_tunnel(segs: u8, thickness: f64) -> Tunnel {
        let mut tunnel = saucer_tunnel(segs, thickness);
        tunnel.anims[0].target = AnimationTarget::Spin;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::ONE)),
            &mut NoopEmitter,
        );
        tunnel
    }

    /// Render a saucer tunnel with few thin segments and spin animation.
    pub fn saucer_few_thin_spin_snapshot() -> LayerCollection {
        snapshot(render_default(&saucer_spin_tunnel(12, 0.1)))
    }

    /// Render a saucer tunnel with many thick segments and spin animation.
    pub fn saucer_many_thick_spin_snapshot() -> LayerCollection {
        snapshot(render_default(&saucer_spin_tunnel(126, 0.5)))
    }

    /// Render a saucer tunnel on a wide ellipse with spin animation.
    pub fn saucer_wide_ellipse_spin_snapshot() -> LayerCollection {
        let mut tunnel = saucer_spin_tunnel(12, 0.1);
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.75)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a saucer tunnel on a tall ellipse with spin animation.
    pub fn saucer_tall_ellipse_spin_snapshot() -> LayerCollection {
        let mut tunnel = saucer_spin_tunnel(12, 0.1);
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.25)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Create an arc tunnel with spin animation on the ellipse path.
    fn arc_spin_tunnel(segs: u8) -> Tunnel {
        let mut tunnel = Tunnel::default();
        set_segments(&mut tunnel, segs);
        tunnel.anims[0].target = AnimationTarget::Spin;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::new(0.5))),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::NPeriods(1)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        tunnel
    }

    /// Many small arc segments with spin — dashes rotate like the line version.
    pub fn arc_spin_many_snapshot() -> LayerCollection {
        snapshot(render_default(&arc_spin_tunnel(126)))
    }

    /// Few large arc segments with spin — curvature visible when rotated.
    pub fn arc_spin_few_snapshot() -> LayerCollection {
        snapshot(render_default(&arc_spin_tunnel(12)))
    }

    /// Wide ellipse with few arcs and spin — exaggerated curvature effect.
    pub fn arc_spin_wide_ellipse_snapshot() -> LayerCollection {
        let mut tunnel = arc_spin_tunnel(12);
        tunnel.handle_state_change(
            StateChange::AspectRatio(UnipolarFloat::new(0.75)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a line-path tunnel in arc mode for snapshot testing.
    pub fn default_tunnel_line_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 24);
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a line-path tunnel in dot mode for snapshot testing.
    pub fn default_tunnel_line_dot_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Dot,
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 24);
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a saucer tunnel with few thin segments on a line path for snapshot testing.
    pub fn saucer_line_few_thin_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Saucer,
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 12);
        tunnel.handle_state_change(
            StateChange::Thickness(UnipolarFloat::new(0.1)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render an arc tunnel on a line path with spin animation.
    pub fn arc_line_spin_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 126);
        tunnel.anims[0].target = AnimationTarget::Spin;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::new(0.5))),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::NPeriods(1)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Render a saucer tunnel on a line path with spin animation.
    pub fn saucer_line_spin_snapshot() -> LayerCollection {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Saucer,
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 12);
        tunnel.handle_state_change(
            StateChange::Thickness(UnipolarFloat::new(0.1)),
            &mut NoopEmitter,
        );
        tunnel.anims[0].target = AnimationTarget::Spin;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::ONE)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        snapshot(render_default(&tunnel))
    }

    /// Create a line-path tunnel with a sine wave animation on AspectRatio.
    /// n_periods=1, moderate amplitude, default sine waveform.
    fn line_aspect_ratio_anim_tunnel(render_mode: RenderMode) -> Tunnel {
        let mut tunnel = Tunnel {
            render_mode,
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 126);
        tunnel.anims[0].target = AnimationTarget::AspectRatio;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::new(0.25))),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::NPeriods(1)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);
        tunnel
    }

    /// Line-path arc tunnel with aspect ratio sine animation.
    pub fn line_aspect_ratio_anim_arc_snapshot() -> LayerCollection {
        snapshot(render_default(&line_aspect_ratio_anim_tunnel(
            RenderMode::Arc,
        )))
    }

    /// Line-path dot tunnel with aspect ratio sine animation.
    pub fn line_aspect_ratio_anim_dot_snapshot() -> LayerCollection {
        snapshot(render_default(&line_aspect_ratio_anim_tunnel(
            RenderMode::Dot,
        )))
    }

    /// Line-path saucer tunnel with aspect ratio sine animation.
    pub fn line_aspect_ratio_anim_saucer_snapshot() -> LayerCollection {
        snapshot(render_default(&line_aspect_ratio_anim_tunnel(
            RenderMode::Saucer,
        )))
    }

    /// Render a sequence of frames of a line-saucer tunnel with marquee motion,
    /// for evaluating edge transition behavior.
    pub fn saucer_line_marquee_sequence() -> Vec<LayerCollection> {
        let mut tunnel = Tunnel {
            render_mode: RenderMode::Saucer,
            shape_mode: ShapeMode::Line,
            ..Default::default()
        };
        set_segments(&mut tunnel, 16);
        tunnel.handle_state_change(
            StateChange::Thickness(UnipolarFloat::new(0.15)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::MarqueeSpeed(BipolarFloat::new(0.5)),
            &mut NoopEmitter,
        );
        tunnel.update_state(Duration::from_secs(1), UnipolarFloat::ZERO);

        let frame_interval = Duration::from_millis(25);
        let frames_per_snapshot = 16;
        let n_snapshots = 24;
        let mut snapshots = Vec::new();
        for _ in 0..n_snapshots {
            let arcs = tunnel.render(
                UnipolarFloat::ONE,
                false,
                RenderContext {
                    clocks: &ClockBank::default().as_static(),

                    palette: &ColorPalette::default(),

                    positions: &PositionBank::default(),

                    audio_envelope: UnipolarFloat::ZERO,
                },
            );
            snapshots.push(vec![arcs]);
            for _ in 0..frames_per_snapshot {
                tunnel.update_state(frame_interval, UnipolarFloat::ZERO);
            }
        }
        snapshots
    }

    /// Render a stress-configured tunnel evolved by 20 frames for snapshot testing.
    pub fn stress_tunnel_evolved_snapshot() -> LayerCollection {
        let frame_interval = Duration::from_micros(25_300);
        let n_frames: u64 = 20;

        let mut tunnel = Tunnel::default();
        configure_stress(&mut tunnel, BipolarFloat::new(-1.0));
        for _ in 0..n_frames {
            tunnel.update_state(frame_interval, UnipolarFloat::ZERO);
        }
        let arcs = tunnel.render(
            UnipolarFloat::ONE,
            false,
            RenderContext {
                clocks: &ClockBank::default().as_static(),

                palette: &ColorPalette::default(),

                positions: &PositionBank::default(),

                audio_envelope: UnipolarFloat::ZERO,
            },
        );
        vec![arcs]
    }

    /// Every target an animation can be pointed at, in the order slots are
    /// handed them.
    ///
    /// Written out rather than taken from `AnimationTarget::VARIANTS` because
    /// which slot draws which target is what the recorded renders are of. The
    /// length comes from the enum, so a target added to it fails to build here
    /// rather than going quietly undrawn.
    const TARGETS: [AnimationTarget; AnimationTarget::VARIANTS.len()] = [
        AnimationTarget::Size,
        AnimationTarget::Thickness,
        AnimationTarget::ColorSaturation,
        AnimationTarget::PositionX,
        AnimationTarget::Rotation,
        AnimationTarget::MarqueeRotation,
        AnimationTarget::AspectRatio,
        AnimationTarget::Color,
        AnimationTarget::ColorSpread,
        AnimationTarget::PositionY,
        AnimationTarget::Spin,
    ];

    /// Every shape mode that distributes segments along a path, in the order
    /// channels are handed them.
    ///
    /// Unlike `TARGETS` and `WAVEFORMS` the length is written out rather than
    /// taken from the enum: the modes that fill an area instead are configured
    /// by `configure_figure`, which reads a different half of the controls.
    const SEGMENT_SHAPE_MODES: [ShapeMode; 2] = [ShapeMode::Ellipse, ShapeMode::Line];

    /// Every way of painting a figure, in the order channels are handed them.
    ///
    /// Written out rather than taken from `DrawMode::VARIANTS` for the same
    /// reason as `TARGETS`, and its length taken from the enum for the same
    /// reason.
    const DRAW_MODES: [DrawMode; DrawMode::VARIANTS.len()] =
        [DrawMode::Fill, DrawMode::Outline, DrawMode::Both];

    /// Every coordinate of a figure that can index its colour ramp, in the
    /// order channels are handed them.
    const COLOR_PHASES: [ColorPhase; ColorPhase::VARIANTS.len()] =
        [ColorPhase::Angle, ColorPhase::Radius, ColorPhase::Linear];

    /// Every waveform an animation can be shaped by, in the order slots are
    /// handed them.
    ///
    /// Written out for the same reason as `TARGETS`, and its length taken from
    /// the enum for the same reason.
    const WAVEFORMS: [Waveform; Waveform::VARIANTS.len()] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Sawtooth,
        Waveform::Square,
        Waveform::Noise,
        Waveform::Constant,
    ];

    /// Configure a tunnel to vary as much as a tunnel can: full colour spread,
    /// no blacking, both integrated angles turning, and every animation slot
    /// spent on its own target, waveform and set of shaping flags.
    ///
    /// `index` of `of` separates one such tunnel from another, so that no two
    /// tunnels configured this way draw the same shapes and so that a mixerful
    /// of them draws every render mode, every shape mode, every animation
    /// target and every waveform at least once.
    pub fn configure_max_variation(tunnel: &mut Tunnel, index: usize, of: usize, segments: u8) {
        let phase = index as f64 / of as f64;
        set_segments(tunnel, segments);
        tunnel.handle_state_change(StateChange::Blacking(KNOB_CENTRE), &mut NoopEmitter);
        tunnel.handle_state_change(
            StateChange::ColorSpread(UnipolarFloat::ONE),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorWidth(UnipolarFloat::new(0.5)),
            &mut NoopEmitter,
        );
        // Halfway up, so that a saturation animation is visible in both
        // directions rather than clamping against zero.
        tunnel.handle_state_change(
            StateChange::ColorSaturation(UnipolarFloat::new(0.5)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::MarqueeSpeed(BipolarFloat::new(-1.0 + 2.0 * phase)),
            &mut NoopEmitter,
        );
        // The rotation and spin angles are integrated from these speeds and sit
        // at exactly zero until the speeds do not, so both stay away from it.
        tunnel.handle_state_change(
            StateChange::RotationSpeed(BipolarFloat::new(0.25 + 0.5 * phase)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::SpinSpeed(BipolarFloat::new(-1.0 + 0.5 * phase)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::RenderModeButton((index % RenderMode::VARIANTS.len()) as u8),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ShapeMode(SEGMENT_SHAPE_MODES[index % SEGMENT_SHAPE_MODES.len()]),
            &mut NoopEmitter,
        );

        for (i, anim) in tunnel.anims.iter_mut().enumerate() {
            configure_varied_animation(anim, index * N_ANIM + i);
        }
    }

    /// Configure a tunnel to draw a filled figure of `library` rather than a
    /// run of segments, spread by `index` of `of` so that no two draw the same
    /// figure the same way.
    ///
    /// Every control a figure reads and a run of segments does not is moved off
    /// its default: which figure of the library, how much of it is painted,
    /// which of its coordinates indexes the colour ramp, and both halves of the
    /// animation split — the targets resolved into the layer and the targets
    /// that travel with it unresolved.
    pub fn configure_figure(tunnel: &mut Tunnel, library: FigureLibrary, index: usize, of: usize) {
        let phase = index as f64 / of as f64;
        tunnel.handle_state_change(
            StateChange::ShapeMode(match library {
                FigureLibrary::Baked => ShapeMode::Sprite,
                FigureLibrary::Generated => ShapeMode::Generated,
            }),
            &mut NoopEmitter,
        );
        // In a figure mode these three name a family of whichever library the
        // mode draws from, a figure in it, and the second degree of freedom a
        // generated family resolves in its own way. Spreading them is what
        // makes every channel draw a different figure.
        tunnel.handle_state_change(
            StateChange::Segments((f64::from(KNOB_MAX) * phase) as u8),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::Blacking((f64::from(KNOB_MAX) * phase) as u8),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::MarqueeSpeed(BipolarFloat::new(-1.0 + 2.0 * phase)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::DrawMode(DRAW_MODES[index % DRAW_MODES.len()]),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorPhase(COLOR_PHASES[index % COLOR_PHASES.len()]),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorSpread(UnipolarFloat::ONE),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorWidth(UnipolarFloat::new(0.5)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::ColorSaturation(UnipolarFloat::new(0.5)),
            &mut NoopEmitter,
        );
        // Thin enough that an outline reads as a contour rather than as a
        // second fill.
        tunnel.handle_state_change(
            StateChange::Thickness(UnipolarFloat::new(0.05)),
            &mut NoopEmitter,
        );
        // The rotation and spin angles are integrated from these speeds and sit
        // at exactly zero until the speeds do not, so both stay away from it.
        tunnel.handle_state_change(
            StateChange::RotationSpeed(BipolarFloat::new(0.25 + 0.5 * phase)),
            &mut NoopEmitter,
        );
        tunnel.handle_state_change(
            StateChange::SpinSpeed(BipolarFloat::new(-1.0 + 0.5 * phase)),
            &mut NoopEmitter,
        );

        for (i, anim) in tunnel.anims.iter_mut().enumerate() {
            configure_varied_animation(anim, index * N_ANIM + i);
        }
    }

    /// Configure one animation slot, spread by `slot` so that consecutive slots
    /// differ in target, waveform, period, rate, duty cycle, smoothing and each
    /// of the three shaping flags.
    fn configure_varied_animation(anim: &mut TargetedAnimation, slot: usize) {
        use AnimStateChange::*;
        anim.target = TARGETS[slot % TARGETS.len()];
        let mut set = |sc| {
            anim.animation
                .control(AnimControlMessage::Set(sc), &mut NoopEmitter)
        };
        set(Waveform(WAVEFORMS[slot % WAVEFORMS.len()]));
        // Never zero: noise reads its smoothing as a cross-correlation term
        // only where the period count is not.
        set(NPeriods(1 + (slot % 4) as u16));
        // Never zero either, or the animation contributes nothing and stops
        // advancing its own clock.
        set(Size(UnipolarFloat::new(0.5)));
        set(Speed(BipolarFloat::new(
            -1.0 + 2.0 * (slot % 7) as f64 / 7.0,
        )));
        set(DutyCycle(UnipolarFloat::new(
            1.0 - 0.05 * (slot % 3) as f64,
        )));
        set(Smoothing(UnipolarFloat::new((slot % 5) as f64 / 5.0)));
        set(Pulse(slot.is_multiple_of(2)));
        set(Standing(slot.is_multiple_of(3)));
        set(Invert(slot.is_multiple_of(5)));
    }

    /// Point a tunnel at shared frame state, so that its hue, its centre and
    /// the timing and amplitude of its animations all come from the palette,
    /// the position bank, the clock bank and the audio envelope rather than
    /// from the tunnel's own settings.
    ///
    /// One slot is held back on its own internal clock, and of the slots that
    /// do follow a show clock, half leave their own audio-size flag clear so
    /// that the clock's flag is the only thing that can scale them by the
    /// envelope.
    pub fn bind_to_frame_state(
        tunnel: &mut Tunnel,
        palette: ColorPaletteIdx,
        position: PositionIdx,
        clock: ClockIdx,
    ) {
        tunnel.handle_state_change(
            StateChange::PaletteSelection(Some(palette)),
            &mut NoopEmitter,
        );
        // The position selection has no control message of its own, so a
        // fixture reaches the field directly.
        tunnel.position_selection = Some(position);
        let internal_slot = clock.0 % N_ANIM;
        for (i, anim) in tunnel.anims.iter_mut().enumerate() {
            let internal = i == internal_slot;
            anim.animation.control(
                AnimControlMessage::SetClockSource(if internal { None } else { Some(clock) }),
                &mut NoopEmitter,
            );
            anim.animation.control(
                AnimControlMessage::Set(AnimStateChange::UseAudioSize(
                    internal || i.is_multiple_of(2),
                )),
                &mut NoopEmitter,
            );
            if internal {
                // Nothing else reaches the internal clock's own audio flag.
                anim.animation.control(
                    AnimControlMessage::Set(AnimStateChange::UseAudioSpeed(true)),
                    &mut NoopEmitter,
                );
            }
        }
    }

    /// The figures the render fixtures draw, by the id the build assigns them.
    ///
    /// The ids come from the shape directory's own order, so a test that
    /// draws one should check the name it got: a figure added to a family
    /// renumbers everything after it, and a golden image would otherwise
    /// quietly become an image of something else.
    pub const SNOWFLAKE: SpriteId = SpriteId(25);
    pub const BULLSEYE: SpriteId = SpriteId(42);
    /// A figure that carves its six sectors with one self-intersecting contour
    /// returning to a single shared point, rather than with a subpath each.
    /// That shared point is where a winding rule is most easily upset by a
    /// coordinate that has moved, which is why this one is drawn.
    pub const PINWHEEL: SpriteId = SpriteId(2);
    /// A figure whose handle is one long straight contour passing close to the
    /// origin, which is where a stroke's colour is hardest to get right.
    pub const UMBRELLA: SpriteId = SpriteId(58);

    /// A tunnel that draws a figure instead of a run of segments.
    ///
    /// Saturated, so a colour knob shows up at all: the default is white.
    fn sprite_tunnel(sprite: SpriteId) -> Tunnel {
        Tunnel {
            shape_mode: ShapeMode::Sprite,
            sprite,
            col_sat: UnipolarFloat::ONE,
            col_center: UnipolarFloat::new(0.55),
            ..Default::default()
        }
    }

    /// A figure in one colour, which is the path that skips the ramp entirely.
    pub fn sprite_flat_snapshot() -> LayerCollection {
        snapshot(render_default(&sprite_tunnel(SNOWFLAKE)))
    }

    /// A figure with a colour sweep along one of its coordinates.
    ///
    /// Three cycles rather than one, so the ramp's wrap and the seam where
    /// angular phase jumps are both in the picture.
    pub fn sprite_color_snapshot(phase: ColorPhase) -> LayerCollection {
        let mut tunnel = sprite_tunnel(SNOWFLAKE);
        tunnel.col_width = UnipolarFloat::ONE;
        tunnel.col_spread = UnipolarFloat::new(3.0 / COLOR_SPREAD_SCALE);
        tunnel.color_phase = phase;
        snapshot(render_default(&tunnel))
    }

    /// A colour animation over a figure that already carries a colour sweep.
    ///
    /// The two have to be independent: the sweep repeats across the figure as
    /// many times as the spread knob asks, and the animation runs the number of
    /// periods *it* was set to over the whole figure, the way an animation runs
    /// over a whole beam. An animation whose period came out as the colour's
    /// would put three saturation lobes here instead of one, and nothing else
    /// in the suite would see it.
    pub fn sprite_color_animation_snapshot() -> LayerCollection {
        let mut tunnel = sprite_tunnel(SNOWFLAKE);
        tunnel.col_width = UnipolarFloat::ONE;
        tunnel.col_spread = UnipolarFloat::new(3.0 / COLOR_SPREAD_SCALE);
        tunnel.anims[0].target = AnimationTarget::ColorSaturation;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Waveform(Waveform::Sine)),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::NPeriods(1)),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::ONE)),
            &mut NoopEmitter,
        );
        snapshot(render_default(&tunnel))
    }

    /// A figure whose sectors all meet at one point.
    ///
    /// Six subpath runs share a single vertex, so every one of them is decided
    /// by the winding at that vertex. A figure built this way is the first
    /// thing to lose a region if the points it is tessellated from are not the
    /// points the library holds.
    pub fn sprite_shared_vertex_snapshot() -> LayerCollection {
        let mut tunnel = sprite_tunnel(PINWHEEL);
        tunnel.col_sat = UnipolarFloat::ZERO;
        snapshot(render_default(&tunnel))
    }

    /// A figure sheared by the spin knob: centre pinned, rim carrying the turn.
    pub fn sprite_spin_snapshot() -> LayerCollection {
        let mut tunnel = sprite_tunnel(SNOWFLAKE);
        tunnel.spin_speed = BipolarFloat::new(0.25);
        snapshot(render_default(&tunnel))
    }

    /// A figure deformed by a radial animation running around its angle,
    /// which is what turns an outline into petals.
    pub fn sprite_radial_animation_snapshot() -> LayerCollection {
        let mut tunnel = sprite_tunnel(BULLSEYE);
        // Small enough that the deformation stays inside the frame: a golden
        // clipped by the viewport hides whatever it clipped.
        tunnel.size = Smoother::new(
            UnipolarFloat::new(0.3),
            Tunnel::GEOM_SMOOTH_TIME,
            SmoothMode::Linear,
        );
        tunnel.anims[0].target = AnimationTarget::Size;
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Waveform(Waveform::Sine)),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::NPeriods(6)),
            &mut NoopEmitter,
        );
        tunnel.anims[0].animation.control(
            AnimControlMessage::Set(AnimStateChange::Size(UnipolarFloat::new(0.4))),
            &mut NoopEmitter,
        );
        snapshot(render_default(&tunnel))
    }

    /// A masked figure stacked over a lit one, which intersects their
    /// apertures the way stacking gobos does.
    pub fn sprite_masked_stack_snapshot() -> LayerCollection {
        let mut lit = sprite_tunnel(SNOWFLAKE);
        lit.col_width = UnipolarFloat::ONE;
        lit.col_spread = UnipolarFloat::new(2.0 / COLOR_SPREAD_SCALE);

        let mut mask = sprite_tunnel(BULLSEYE);
        mask.size = Smoother::new(
            UnipolarFloat::new(0.35),
            Tunnel::GEOM_SMOOTH_TIME,
            SmoothMode::Linear,
        );

        vec![render_default(&lit), render_masked(&mask)]
    }

    /// A stroked outline carrying a colour sweep.
    ///
    /// The umbrella's handle is a single straight contour running close to the
    /// origin, where angular phase moves fastest — so if a stroke's colour
    /// were taken from where its vertices landed rather than from the contour,
    /// this is the figure it would show on.
    pub fn sprite_outline_color_snapshot() -> LayerCollection {
        let mut tunnel = sprite_tunnel(UMBRELLA);
        tunnel.draw_mode = DrawMode::Outline;
        tunnel.col_width = UnipolarFloat::ONE;
        // The most cycles the knob can ask for, which is where a stroke
        // sampled too coarsely along its length would band first.
        tunnel.col_spread = UnipolarFloat::ONE;
        tunnel.thickness = Smoother::new(
            UnipolarFloat::new(0.05),
            Tunnel::GEOM_SMOOTH_TIME,
            SmoothMode::Linear,
        );
        snapshot(render_default(&tunnel))
    }

    /// A figure's contours stroked instead of its interior filled.
    pub fn sprite_outline_snapshot() -> LayerCollection {
        let mut tunnel = sprite_tunnel(SNOWFLAKE);
        tunnel.draw_mode = DrawMode::Outline;
        tunnel.thickness = Smoother::new(
            UnipolarFloat::new(0.05),
            Tunnel::GEOM_SMOOTH_TIME,
            SmoothMode::Linear,
        );
        snapshot(render_default(&tunnel))
    }

    /// A figure of the generated library, by the family and the two numbers
    /// that place it inside that family.
    fn generated_id(family: ShapeFamily, arity: u32, secondary: f64) -> GeneratedId {
        GeneratedId {
            family,
            arity: Arity::new(arity),
            secondary: Secondary::new(secondary),
        }
    }

    /// A seven-pointed star, which is the plainest figure the generated
    /// library makes.
    pub fn generated_star() -> GeneratedId {
        generated_id(ShapeFamily::StarPolygon, 7, 0.0)
    }

    /// Two sets of parallel bars cut to a disc, at a small angle to each other.
    ///
    /// Where a bar of one set crosses a bar of the other the two cancel, and
    /// the lens-shaped holes that leaves are the beat the figure is made of.
    /// They are holes only under an even-odd fill; under a non-zero one the
    /// crossings fill solid and the beat is gone. So this is the figure that
    /// says the fill rule survived the trip out of the frame the family was
    /// built in.
    pub fn generated_moire() -> GeneratedId {
        generated_id(ShapeFamily::MoireGrid, 8, 0.0)
    }

    /// A star lattice at its widest reach.
    ///
    /// The family that runs furthest outside the frame it is built in, and so
    /// the one that says a generated figure is mapped at a fixed scale rather
    /// than fitted to what it happens to reach. Fitted, it would be a small
    /// object in the middle of the frame instead of a field the frame cuts.
    pub fn generated_lattice() -> GeneratedId {
        generated_id(ShapeFamily::StarLattice, 1, 1.0)
    }

    /// A tunnel that draws a generated figure.
    ///
    /// Saturated, so a colour knob shows up at all: the default is white.
    fn generated_tunnel(generated: GeneratedId) -> Tunnel {
        Tunnel {
            shape_mode: ShapeMode::Generated,
            generated,
            col_sat: UnipolarFloat::ONE,
            col_center: UnipolarFloat::new(0.55),
            ..Default::default()
        }
    }

    /// A generated figure in one colour, which is the path that skips the ramp
    /// entirely and draws the tessellator's own triangles.
    pub fn generated_flat_snapshot() -> LayerCollection {
        snapshot(render_default(&generated_tunnel(generated_star())))
    }

    /// A figure whose contours cancel where they cross.
    pub fn generated_even_odd_snapshot() -> LayerCollection {
        snapshot(render_default(&generated_tunnel(generated_moire())))
    }

    /// A figure that reaches well outside the frame it was built in, which
    /// the viewport ends rather than any clip in the geometry.
    pub fn generated_field_snapshot() -> LayerCollection {
        snapshot(render_default(&generated_tunnel(generated_lattice())))
    }

    /// A generated figure with a colour sweep along its angle, which is the
    /// meshed and ramped path a baked figure takes.
    pub fn generated_color_snapshot() -> LayerCollection {
        let mut tunnel = generated_tunnel(generated_star());
        tunnel.col_width = UnipolarFloat::ONE;
        tunnel.col_spread = UnipolarFloat::new(3.0 / COLOR_SPREAD_SCALE);
        snapshot(render_default(&tunnel))
    }

    /// A generated figure's contours stroked instead of its interior filled.
    pub fn generated_outline_snapshot() -> LayerCollection {
        let mut tunnel = generated_tunnel(generated_star());
        tunnel.draw_mode = DrawMode::Outline;
        tunnel.thickness = Smoother::new(
            UnipolarFloat::new(0.05),
            Tunnel::GEOM_SMOOTH_TIME,
            SmoothMode::Linear,
        );
        snapshot(render_default(&tunnel))
    }

    fn render_masked(tunnel: &Tunnel) -> Layer {
        tunnel.render(
            UnipolarFloat::ONE,
            true,
            RenderContext {
                clocks: &ClockBank::default().as_static(),
                palette: &ColorPalette::default(),
                positions: &PositionBank::default(),
                audio_envelope: UnipolarFloat::ZERO,
            },
        )
    }
}
