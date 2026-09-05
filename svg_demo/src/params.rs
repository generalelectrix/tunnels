//! Parameters the control window publishes and the render window consumes.

use serde::{Deserialize, Serialize};

/// How many shape layers the demo stacks.
pub const N_LAYERS: usize = 3;

/// Localhost UDP port the control window publishes parameters on.
pub const PORT: u16 = 47823;

/// Which coordinate on the shape drives the color sawtooth.
///
/// A tunnel's color model is a sawtooth over `rel_angle` — a segment's position
/// as a fraction of the way around the ring. A closed SVG figure has no
/// segments, so this picks what stands in for that fraction. `Angle` is the
/// literal analogue: it is the same quantity a tunnel uses, which means
/// rotating the shape sweeps its color exactly as rotating a tunnel does.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPhase {
    /// Angle about the shape's center. The direct analogue of `rel_angle`.
    Angle,
    /// Distance from the shape's center — concentric bands.
    Radius,
    /// Position across the shape's x axis.
    LinearX,
    /// Position along the shape's y axis.
    LinearY,
}

impl ColorPhase {
    pub const ALL: [Self; 4] = [Self::Angle, Self::Radius, Self::LinearX, Self::LinearY];

    pub fn label(self) -> &'static str {
        match self {
            Self::Angle => "angle",
            Self::Radius => "radius",
            Self::LinearX => "linear x",
            Self::LinearY => "linear y",
        }
    }
}

/// Whether a layer draws the shape's interior, its outline, or both.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawMode {
    Fill,
    Outline,
    Both,
}

impl DrawMode {
    pub const ALL: [Self; 3] = [Self::Fill, Self::Outline, Self::Both];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Outline => "outline",
            Self::Both => "both",
        }
    }

    pub fn draws_fill(self) -> bool {
        matches!(self, Self::Fill | Self::Both)
    }

    pub fn draws_outline(self) -> bool {
        matches!(self, Self::Outline | Self::Both)
    }
}

/// One shape, its placement, and how it is colored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LayerParams {
    pub enabled: bool,
    /// Index into the loaded shape library.
    pub shape: usize,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Static rotation, in turns.
    pub rotation: f64,
    /// Continuous rotation, in turns per second.
    pub spin_speed: f64,
    pub shear_x: f64,
    pub shear_y: f64,
    /// Position offset, in units of the smaller screen dimension.
    pub x: f64,
    pub y: f64,
    pub draw_mode: DrawMode,
    /// Outline width, in shape-normalized units.
    pub stroke_width: f64,

    // Color, ported from `Tunnel`: hue is `col_center` plus a sawtooth over a
    // spatial phase, with `col_width` as the excursion and `col_spread` picking
    // an integer number of cycles. Value is fixed at 1.0 in the real system —
    // brightness is `level`, which becomes the alpha channel.
    pub color_phase: ColorPhase,
    pub col_center: f64,
    pub col_width: f64,
    pub col_spread: f64,
    pub col_sat: f64,
    /// Alpha, and the only brightness control — matches `Channel.level`.
    pub level: f64,
    /// Paint opaque black instead of color, punching a hole in everything below.
    /// This is the `Channel.mask` behavior from the real mixer.
    pub mask: bool,
}

impl Default for LayerParams {
    fn default() -> Self {
        Self {
            enabled: false,
            shape: 0,
            scale_x: 0.8,
            scale_y: 0.8,
            rotation: 0.0,
            spin_speed: 0.0,
            shear_x: 0.0,
            shear_y: 0.0,
            x: 0.0,
            y: 0.0,
            draw_mode: DrawMode::Fill,
            stroke_width: 0.02,
            color_phase: ColorPhase::Angle,
            col_center: 0.55,
            col_width: 0.0,
            col_spread: 0.0,
            col_sat: 0.8,
            level: 1.0,
            mask: false,
        }
    }
}

/// A full frame of control state.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DemoParams {
    pub layers: Vec<LayerParams>,
}

impl Default for DemoParams {
    fn default() -> Self {
        let mut layers = vec![LayerParams::default(); N_LAYERS];
        layers[0].enabled = true;
        Self { layers }
    }
}

/// How many hue cycles a full turn of `col_spread` buys. Matches
/// `COLOR_SPREAD_SCALE` in `tunnels/src/tunnel.rs`.
pub const COLOR_SPREAD_SCALE: f64 = 16.0;

/// The scaled spatial phase driving a layer's color sawtooth.
///
/// Depends only on which coordinate the color follows and how many cycles it
/// spans — not on hue, width, saturation or level. That is what lets a mesh
/// refined against it survive those knobs moving.
#[derive(Copy, Clone, PartialEq)]
pub struct PhaseField {
    phase: ColorPhase,
    cycles: f32,
}

impl PhaseField {
    pub fn of(layer: &LayerParams) -> Self {
        Self {
            phase: layer.color_phase,
            cycles: (COLOR_SPREAD_SCALE * layer.col_spread).floor() as f32,
        }
    }

    /// The phase at a point in shape space, scaled by the cycle count.
    ///
    /// Shape space is the normalised unit box, so this rides with the figure
    /// rather than being pinned to the screen.
    pub fn at(self, p: [f32; 2]) -> f32 {
        let (x, y) = (p[0], p[1]);
        let unit = match self.phase {
            // The direct analogue of a tunnel segment's `rel_angle`.
            ColorPhase::Angle => y.atan2(x) / std::f32::consts::TAU,
            // The far corner of a unit box is at sqrt(2); dividing by that
            // keeps a full sweep inside one cycle.
            ColorPhase::Radius => (x * x + y * y).sqrt() / std::f32::consts::SQRT_2,
            ColorPhase::LinearX => (x + 1.0) / 2.0,
            ColorPhase::LinearY => (y + 1.0) / 2.0,
        };
        unit * self.cycles
    }

}
