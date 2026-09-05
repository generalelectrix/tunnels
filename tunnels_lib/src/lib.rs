//! Code shared between the tunnels console and client.

pub mod bootstrap;
pub mod color;
pub mod notified;
pub mod number;
pub mod prompt;
pub mod repaint;
pub mod smooth;
pub mod transient_indicator;

use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A helper wrapper around an atomically-reference-counted atomic boolean.
/// Used to control program flow across multiple threads.
#[derive(Debug, Clone)]
pub struct RunFlag(Arc<AtomicBool>);

impl Default for RunFlag {
    fn default() -> Self {
        RunFlag(Arc::new(AtomicBool::new(true)))
    }
}

impl RunFlag {
    /// Return true if the program should continue.
    pub fn should_run(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Command the program to stop.
    pub fn stop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Controls how a shape is rendered.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub enum RenderMode {
    /// Render as an arc segment (default).
    #[default]
    Arc,
    /// Render as a filled circle positioned at the arc segment centroid.
    Dot,
    /// Render as a filled ellipse at the arc centroid, sized by chord and thickness.
    Saucer,
}

/// Controls the geometric path that segments are distributed along.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
pub enum PathShape {
    /// Segments are distributed along an ellipse (default).
    #[default]
    Ellipse,
    /// Segments are distributed along a straight line.
    Line,
}

/// A command to draw a single shape, less the render mode and path shape that
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
/// The render mode and path shape apply to every shape in the layer, which is
/// what makes a layer the unit a renderer can dispatch on once instead of per
/// shape.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Layer {
    pub render_mode: RenderMode,
    pub path_shape: PathShape,
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
        path_shape: PathShape,
        span: f64,
        shapes: Vec<ShapeGeometry>,
    ) -> Self {
        Self {
            render_mode,
            path_shape,
            span,
            shapes,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
}

pub type LayerCollection = Vec<Arc<Layer>>;

const ALMOST_EQ_TOLERANCE: f64 = 0.000_000_1;

/// Whether two values agree to within the tolerance a test cares about.
#[inline(always)]
pub fn almost_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < ALMOST_EQ_TOLERANCE
}

/// Panic unless two values agree to within the tolerance a test cares about.
pub fn assert_almost_eq(a: f64, b: f64) {
    assert!(almost_eq(a, b), "{a} != {b}");
}
