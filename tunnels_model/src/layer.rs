//! The drawable geometry a beam expands into.
//!
//! This is the far end of the model: everything above it describes a show,
//! and everything here describes shapes on a screen.

use crate::animation::{PreparedAnimation, TargetedAnimation};
use crate::waveforms::{WaveformArgs, sawtooth};
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use tunnels_lib::number::{Phase, UnipolarFloat};
use tunnels_shapes::{Arity, Secondary, ShapeFamily};

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

    /// The library this mode's figure comes from, or `None` if it draws
    /// segments instead of filling an area.
    ///
    /// The other half of the pair [`ShapeMode::segment_path`] opens: a mode
    /// either distributes marks along a path or names a figure in a library,
    /// and which of the two it does decides what every geometry control means.
    pub fn figure_library(self) -> Option<FigureLibrary> {
        match self {
            Self::Ellipse | Self::Line => None,
            Self::Sprite => Some(FigureLibrary::Baked),
            Self::Generated => Some(FigureLibrary::Generated),
        }
    }

    /// Whether this mode draws a run of segments.
    ///
    /// The render mode acts on segments, and means nothing to a mode that
    /// draws none.
    pub fn draws_segments(self) -> bool {
        self.segment_path().is_some()
    }

    /// Whether the marquee knob names anything in this mode.
    ///
    /// It turns a run of marks around a beam, which a mode drawing none has
    /// nothing to do with — except a generated figure, which has a second
    /// degree of freedom and no other free knob to reach it with.
    pub fn reads_marquee_knob(self) -> bool {
        self.draws_segments() || matches!(self, Self::Generated)
    }
}

/// Where a figure mode's figures come from.
///
/// The two libraries are shaped alike for a control to walk — a list of
/// families, each a run of figures — which is what lets one pair of knobs
/// address either.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FigureLibrary {
    /// Figures baked into the build from artwork.
    Baked,
    /// Figures built from a family and the two numbers that place one in it.
    Generated,
}

/// The curve a run of segments is distributed along.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SegmentPath {
    Ellipse,
    Line,
}

/// Which coordinate of a figure indexes the color ramp.
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
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
#[derive(
    Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash, VariantArray,
)]
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

/// Identifies one figure computed on demand.
///
/// A family and the two numbers that place a figure inside it, which is the
/// whole of what builds one. Every other number a family carries follows from
/// these, so this is the figure and not a handle to it.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GeneratedId {
    pub family: ShapeFamily,
    pub arity: Arity,
    pub secondary: Secondary,
}

impl Default for GeneratedId {
    /// The first family at the bottom of its range, which is the figure a beam
    /// draws before any knob has named another.
    fn default() -> Self {
        let family = ShapeFamily::ALL[0];
        Self {
            family,
            arity: Arity::new(*family.arity_range().start()),
            secondary: Secondary::new(0.0),
        }
    }
}

/// Identifies one figure, however it came to exist.
///
/// A figure is drawn the same way whichever half of the library it came from,
/// so this is what the caches between the model and the screen are keyed on:
/// one figure, one set of contours, one mesh per density.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FigureId {
    /// A figure baked into the build from artwork.
    Baked(SpriteId),
    /// A figure built from a family and its two parameters.
    Generated(GeneratedId),
}

/// A command to draw a single shape, less the render mode and segment path
/// that the layer holding it fixes for all of its shapes at once.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct ShapeGeometry {
    pub color: Hsva,
    pub placement: Placement,
    pub thickness: f64,
    /// Where the shape starts along the layer's path, in turns.
    pub start: f64,
    pub spin_angle: f64,
}

/// A run of shapes drawn the same way.
///
/// The render mode and segment path apply to every shape in the layer, which
/// is what makes a layer the unit a renderer can dispatch on once instead of
/// per shape.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SegmentLayer {
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

impl SegmentLayer {
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

/// Where a shape sits, how large it is, and which way it is turned.
///
/// The half-extents mean whatever the shape they place reads them as: the two
/// radii of an ellipse, the half-length and offset of a line, or the box a
/// figure's own unit square is scaled into.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub extent_x: f64,
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

    /// Whether this field masks: opaque black everywhere, whatever is asked of
    /// it.
    ///
    /// Every channel a colour resolves to is scaled by the value, so a field
    /// with no value paints black at any point and under any colour animation
    /// -- hue and saturation are multiplied away before they can reach a
    /// pixel. That is what lets a mask be resolved once instead of per point
    /// or per texel.
    ///
    /// The three adjustments an animation makes -- centre, width, saturation
    /// -- are what this rests on. A target that moved the value would break
    /// it, and there is none.
    pub fn is_mask(&self) -> bool {
        self.val == 0.0
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
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    pub hue: f64,
    pub sat: f64,
    pub val: f64,
    /// Alpha, carrying the channel's own level.
    pub level: f64,
}

/// A figure drawn as an area rather than as a run of segments.
///
/// Carries no geometry: a figure is named rather than described, and the client
/// is where the name becomes contours, so what travels is where to put it and
/// how to colour it.
#[derive(Debug, Clone)]
pub struct FillLayer {
    pub figure: FigureId,
    pub placement: Placement,
    /// The beam's spin knob, as the operator set it.
    pub spin_speed: f64,
    /// Stroke width, in the same units a segment's thickness is.
    pub thickness: f64,
    pub draw_mode: DrawMode,
    pub color: ColorField,
    /// Animations resolved when the colour ramp is built, once per texel.
    pub color_anims: Vec<TargetedAnimation<PreparedAnimation>>,
    /// Animations resolved per point of the figure, displacing it.
    pub warps: Vec<TargetedAnimation<PreparedAnimation>>,
}

/// What a beam expands into for one frame.
#[derive(Debug, Clone)]
pub enum Layer {
    /// A run of segments along a path.
    Segments(SegmentLayer),
    /// A filled figure.
    Fill(FillLayer),
}

impl Layer {
    /// Whether this layer would draw nothing, and so can be dropped before it
    /// reaches a renderer.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Segments(l) => l.shapes.is_empty(),
            // A figure is one shape and is always there; whether the build
            // carries the figure it names is the renderer's question.
            Self::Fill(_) => false,
        }
    }
}

pub type LayerCollection = Vec<Layer>;

#[cfg(test)]
mod test {
    use super::*;

    /// A mask is resolved once for the whole layer rather than once per point.
    /// That is only sound if no adjustment an animation can make reaches the
    /// result.
    #[test]
    fn no_adjustment_moves_what_a_mask_paints() {
        let mask = ColorField {
            phase: ColorPhase::Angle,
            cycles: 0.,
            center: 0.,
            width: 0.,
            sat: 0.,
            val: 0.,
            level: 1.,
        };
        assert!(mask.is_mask());

        let flat = mask.sample(Phase::ZERO, ColorAdjust::default());
        for adjust in [
            ColorAdjust {
                center: 0.4,
                width: 0.,
                sat: 0.,
            },
            ColorAdjust {
                center: 0.,
                width: 1.,
                sat: 0.,
            },
            ColorAdjust {
                center: 0.,
                width: 0.,
                sat: 1.,
            },
            ColorAdjust {
                center: 0.9,
                width: 1.,
                sat: 1.,
            },
        ] {
            for phase in [0., 0.25, 0.5, 0.75] {
                let sampled = mask.sample(Phase::new(phase), adjust);
                assert_eq!(
                    sampled.val, flat.val,
                    "a mask gained a value from {adjust:?}"
                );
                assert_eq!(
                    sampled.level, flat.level,
                    "a mask changed alpha under {adjust:?}"
                );
            }
        }

        // A field with a value does move, so the test above is not vacuous.
        let lit = ColorField { val: 1., ..mask };
        assert!(!lit.is_mask());
        assert_ne!(
            lit.sample(Phase::ZERO, ColorAdjust::default()).val,
            flat.val
        );
    }
}
