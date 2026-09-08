//! The drawable geometry a beam expands into.
//!
//! This is the far end of the model: everything above it describes a show,
//! and everything here describes shapes on a screen.

use crate::animation::PreparedAnimation;
use crate::animation_target::AnimationTarget;
use crate::waveforms::{WaveformArgs, sawtooth};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use strum::VariantArray;
use tunnels_lib::number::{Phase, UnipolarFloat};

/// Controls how a shape is rendered.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum RenderMode {
    /// Render as an arc segment (default).
    #[default]
    Arc,
    /// Render as a filled circle positioned at the arc segment centroid.
    Dot,
    /// Render as a filled ellipse at the arc centroid, sized by chord and thickness.
    Saucer,
}

/// What a beam draws, and how its geometry parameters are read.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
pub enum ShapeMode {
    /// Segments are distributed along an ellipse (default).
    #[default]
    Ellipse,
    /// Segments are distributed along a straight line.
    Line,
    /// A filled figure computed from the beam's own parameters.
    Generated,
    /// A filled figure baked into the build.
    Sprite,
}

impl ShapeMode {
    /// The path this mode's segments are distributed along, or `None` if it
    /// fills an area instead of drawing segments.
    pub fn segment_path(self) -> Option<SegmentPath> {
        match self {
            Self::Ellipse => Some(SegmentPath::Ellipse),
            Self::Line => Some(SegmentPath::Line),
            Self::Generated | Self::Sprite => None,
        }
    }

    /// Whether this mode draws a run of segments.
    ///
    /// The controls that act on segments -- the marquee and the render mode
    /// -- mean nothing to a mode that draws none.
    pub fn draws_segments(self) -> bool {
        self.segment_path().is_some()
    }
}

/// The curve a run of segments is distributed along.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SegmentPath {
    Ellipse,
    Line,
}

/// Which coordinate of a figure indexes the color ramp.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub enum ColorPhase {
    /// The angle about the figure's center (default).
    #[default]
    Angle,
    /// The distance from the figure's center.
    Radius,
    /// The figure's own vertical coordinate.
    ///
    /// The frame is the figure's rather than the screen's, so this axis turns
    /// with the figure: a ramp along any other axis is this one plus a
    /// rotation, which is why there is only one linear phase.
    Linear,
}

/// How much of a figure is painted.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub enum DrawMode {
    /// The interior only (default).
    #[default]
    Fill,
    /// The contours only, stroked at the beam's thickness.
    Outline,
    /// The interior with its contours stroked over it.
    Both,
}

impl DrawMode {
    pub fn draws_fill(self) -> bool {
        matches!(self, Self::Fill | Self::Both)
    }

    pub fn draws_outline(self) -> bool {
        matches!(self, Self::Outline | Self::Both)
    }
}

/// Identifies one figure baked into the build.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct SpriteId(pub u16);

/// A command to draw a single shape, less the render mode and segment path
/// that the layer holding it fixes for all of its shapes at once.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ShapeGeometry {
    pub level: f64,
    pub thickness: f64,
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    pub x: f64,
    pub y: f64,
    pub extent_x: f64,
    pub extent_y: f64,
    pub start: f64,
    pub rot_angle: f64,
    pub spin_angle: f64,
}

/// A run of shapes drawn the same way.
///
/// The render mode and segment path apply to every shape in the layer, which
/// is what makes a layer the unit a renderer can dispatch on once instead of
/// per shape.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MarkLayer {
    pub render_mode: RenderMode,
    pub segment_path: SegmentPath,
    /// The angular width every segment in this layer spans, in turns.
    ///
    /// A segment's stop angle is its `start` plus this, so a segment that
    /// closes into a full circle spans exactly one turn — a test the renderer
    /// can make without subtracting two nearly equal angles.
    pub span: f64,
    pub shapes: Vec<ShapeGeometry>,
}

impl MarkLayer {
    pub fn new(
        render_mode: RenderMode,
        segment_path: SegmentPath,
        span: f64,
        shapes: Vec<ShapeGeometry>,
    ) -> Self {
        Self {
            render_mode,
            segment_path,
            span,
            shapes,
        }
    }
}

/// Where a figure sits and how large it is, in the units a segment uses.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    /// Half-width, before the figure's own unit box is scaled by it.
    pub extent_x: f64,
    /// Half-height.
    pub extent_y: f64,
    pub rot_angle: f64,
}

/// How a figure's colour is resolved from a coordinate on it.
///
/// This is a tunnel's colour model with a figure's coordinate standing in for
/// the segment index: `hue = center + 0.5 * width * sawtooth(phase * cycles)`.
/// A closed figure has no segments, so [`ColorPhase`] picks what does the
/// indexing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct ColorField {
    pub phase: ColorPhase,
    /// Whole colour cycles across the figure, already floored.
    pub cycles: f64,
    pub center: f64,
    pub width: f64,
    pub sat: f64,
    pub val: f64,
    pub level: f64,
}

impl ColorField {
    /// Whether every point of the figure resolves to the same colour.
    ///
    /// A uniform figure needs no ramp and no interpolation, which is also the
    /// state a mask is in. An animation on the colour can still move that one
    /// colour over time, so this is not on its own a reason to skip the ramp.
    pub fn is_uniform(&self) -> bool {
        self.width == 0.0 || self.cycles == 0.0
    }

    /// The colour at a point of one cycle.
    ///
    /// This is a tunnel's own hue expression with the cycle count taken out:
    /// the count multiplies the coordinate rather than the table, so one cycle
    /// is all a table has to hold and a figure's colour reads the same as a
    /// beam's at the same knob settings.
    pub fn sample(&self, phase: Phase, adjust: ColorAdjust) -> Hsva {
        let hue = Phase::new(
            (self.center + adjust.center)
                + 0.5
                    * (self.width + adjust.width)
                    * sawtooth(&WaveformArgs {
                        phase_spatial: phase,
                        phase_temporal: Phase::ZERO,
                        smoothing: UnipolarFloat::ZERO,
                        duty_cycle: UnipolarFloat::ONE,
                        pulse: false,
                        standing: false,
                    }),
        );
        Hsva {
            hue: hue.val(),
            sat: UnipolarFloat::new(self.sat + adjust.sat).val(),
            val: self.val,
            level: self.level,
        }
    }
}

/// What animations add to a colour before it is resolved.
#[derive(Debug, Clone, Copy, Default)]
pub struct ColorAdjust {
    pub center: f64,
    pub width: f64,
    pub sat: f64,
}

/// A resolved colour, and the level it is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    /// Alpha, carrying the channel's own level.
    pub level: f64,
}

/// An animation resolved for this frame, and the knob it drives.
///
/// The target is the one the operator set, not a translation of it: a figure
/// and a beam share a control surface, so they share the vocabulary that
/// surface speaks. What a target *means* on a figure is decided where the
/// figure is drawn.
#[derive(Debug, Clone, Copy)]
pub struct FillAnimation {
    pub target: AnimationTarget,
    pub animation: PreparedAnimation,
}

/// Identifies one layer across frames, so its caches survive the mixer moving.
///
/// The path of channel indices from the mixer root, rather than the layer's
/// position in the output: `Mixer::render_video_channel` skips channels at
/// level zero, so bringing one up shifts every later index and would hand
/// every fill after it another layer's ramp for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayerKey {
    path: [u16; Self::MAX_DEPTH],
    len: u8,
}

impl LayerKey {
    /// How deep the path is followed.
    ///
    /// Looks may nest far deeper than this. Past it the key stops recording,
    /// so two figures under the same eight-deep prefix share a cache — the
    /// stale-ramp flash this exists to prevent, at a nesting depth no show
    /// reaches, rather than an unbounded key on a per-frame path.
    const MAX_DEPTH: usize = 8;

    /// Descend into a channel, returning the key for what is inside it.
    pub fn child(mut self, channel: usize) -> Self {
        if let (Some(slot), Ok(channel)) = (
            self.path.get_mut(usize::from(self.len)),
            u16::try_from(channel),
        ) {
            *slot = channel;
            self.len += 1;
        }
        self
    }
}

/// A figure drawn as an area rather than as a run of segments.
///
/// Carries no geometry: the figure itself is baked into the build and the
/// client looks it up, so what travels is where to put it and how to colour
/// it.
#[derive(Debug, Clone)]
pub struct FillLayer {
    pub key: LayerKey,
    pub sprite: SpriteId,
    pub placement: Placement,
    /// Winding at the rim, in turns. Bounded rather than integrated: five
    /// turns at the rim stays five turns tighter than one, so an accumulator
    /// would spiral without bound.
    pub spin: f64,
    /// Stroke width, in the same units a segment's thickness is.
    pub thickness: f64,
    pub draw_mode: DrawMode,
    pub color: ColorField,
    /// Animations resolved when the colour ramp is built, once per texel.
    pub color_anims: Vec<FillAnimation>,
    /// Animations resolved per point of the figure, displacing it.
    pub warps: Vec<FillAnimation>,
}

/// What a beam expands into for one frame.
#[derive(Debug, Clone)]
pub enum Layer {
    /// A run of segments along a path.
    Marks(MarkLayer),
    /// A filled figure.
    Fill(FillLayer),
}

impl Layer {
    /// Whether this layer would draw nothing, and so can be dropped before it
    /// reaches a renderer.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Marks(l) => l.shapes.is_empty(),
            // A figure is one shape and is always there; whether the build
            // carries the sprite it names is the renderer's question.
            Self::Fill(_) => false,
        }
    }
}

pub type LayerCollection = Vec<Arc<Layer>>;
