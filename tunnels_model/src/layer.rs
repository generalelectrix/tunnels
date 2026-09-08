//! The drawable geometry a beam expands into.
//!
//! This is the far end of the model: everything above it describes a show,
//! and everything here describes shapes on a screen.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use strum::VariantArray;

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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
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

/// Identifies one figure baked into the build.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub struct SpriteId(pub u16);

/// A command to draw a single shape, less the render mode and shape mode that
/// the layer holding it fixes for all of its shapes at once.
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
/// The render mode and shape mode apply to every shape in the layer, which is
/// what makes a layer the unit a renderer can dispatch on once instead of per
/// shape.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Layer {
    pub render_mode: RenderMode,
    pub shape_mode: ShapeMode,
    /// The angular width every segment in this layer spans, in turns.
    ///
    /// A segment's stop angle is its `start` plus this, so a segment that
    /// closes into a full circle spans exactly one turn — a test the renderer
    /// can make without subtracting two nearly equal angles.
    pub span: f64,
    pub shapes: Vec<ShapeGeometry>,
}

impl Layer {
    pub fn new(
        render_mode: RenderMode,
        shape_mode: ShapeMode,
        span: f64,
        shapes: Vec<ShapeGeometry>,
    ) -> Self {
        Self {
            render_mode,
            shape_mode,
            span,
            shapes,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
}

pub type LayerCollection = Vec<Arc<Layer>>;
