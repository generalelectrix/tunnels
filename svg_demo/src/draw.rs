//! Drawing shape meshes through piston's `Graphics` trait.
//!
//! Generic over the backend so the same code runs on the GL window and on the
//! software rasteriser used for headless contact sheets.

use crate::mesh::RefinedMesh;
use crate::anim::LiveWave;
use crate::params::{AnimTarget, COLOR_SPREAD_SCALE, ColorPhase, LayerParams, PhaseField};
use crate::shapes::ShapeMesh;
use graphics::draw_state::DrawState;
use graphics::math::Matrix2d;
use graphics::{Graphics, Transformed};
use std::f64::consts::TAU;

/// Vertices per chunk handed to the backend. `BACK_END_MAX_VERTEX_COUNT` is
/// 1023, which divides evenly into triangles.
const CHUNK: usize = 1023;

/// Convert HSV to RGB. Matches `tunnelclient::draw::hsv_to_rgb` so colors read
/// the same as they do in the real client.
fn hsv_to_rgb(hue: f64, sat: f64, val: f64, alpha: f64) -> [f32; 4] {
    let rgb = |r: f64, g: f64, b: f64| [r as f32, g as f32, b as f32, alpha as f32];
    if sat == 0.0 {
        return rgb(val, val, val);
    }
    let hue = hue.rem_euclid(1.0);
    let var_h = if hue == 1.0 { 0.0 } else { hue * 6.0 };
    let var_i = var_h.floor();
    let var_1 = val * (1.0 - sat);
    let var_2 = val * (1.0 - sat * (var_h - var_i));
    let var_3 = val * (1.0 - sat * (1.0 - (var_h - var_i)));
    match var_i as i64 {
        0 => rgb(val, var_3, var_1),
        1 => rgb(var_2, val, var_1),
        2 => rgb(var_1, val, var_3),
        3 => rgb(var_1, var_2, val),
        4 => rgb(var_3, var_1, val),
        _ => rgb(val, var_1, var_2),
    }
}


/// Whether every vertex of this layer resolves to the same color, letting the
/// cheaper flat-color path handle it.
pub fn is_uniform(layer: &LayerParams) -> bool {
    layer.col_width == 0.0 || (COLOR_SPREAD_SCALE * layer.col_spread).floor() == 0.0
}

/// The transform placing a layer's unit-box shape on screen.
///
/// `critical` is the smaller screen dimension, matching how the real client
/// scales geometry (`ClientConfig::critical_size`).
pub fn layer_transform(
    base: Matrix2d,
    layer: &LayerParams,
    time: f64,
    critical: f64,
) -> Matrix2d {
    let angle = (layer.rotation + layer.rot_speed * time) * TAU;
    base.trans(layer.x * critical, layer.y * critical)
        .rot_rad(angle)
        .shear(layer.shear_x, layer.shear_y)
        .scale(
            layer.scale_x * critical * 0.5,
            layer.scale_y * critical * 0.5,
        )
}

/// Put two phase coordinates on the same branch.
///
/// Angular phase jumps by a whole turn across the far side of the shape, where
/// `atan2` wraps. Both ends still sample the right texel — the ramp repeats,
/// and a whole number of cycles is a whole number of periods — but interpolating
/// straight between them sweeps the long way round, painting a band of spurious
/// rainbow along the seam.
///
/// The shift has to be in units of the wrap period, not of one cycle. A coarse
/// mesh with a high cycle count has triangles legitimately spanning more than a
/// cycle, and rounding to the nearest cycle would "correct" those into
/// nonsense. A whole turn is the only jump that is ever real.
#[inline]
fn same_branch(reference: f32, u: f32, period: f32) -> f32 {
    u + ((reference - u) / period).round() * period
}

/// How far a spin animation can rotate a point at full size: a quarter turn.
const MAX_SPIN: f32 = std::f32::consts::TAU / 4.0;

/// One animation, paired with the coordinate it runs along.
pub struct AxisWave<'a> {
    pub target: AnimTarget,
    pub phase: ColorPhase,
    pub wave: &'a LiveWave,
}

/// A geometry animation. Same shape as any other; named for where it is used.
pub type GeometryWave<'a> = AxisWave<'a>;

/// Buffers the per-vertex pass fills, reused across frames.
#[derive(Default)]
pub struct VertexBuffers {
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub tints: Vec<[f32; 4]>,
}

/// Everything the per-vertex pass reads.
pub struct VertexWork<'a> {
    /// The coordinate indexing the ramp.
    pub field: PhaseField,
    /// The layer's static spin, in turns at the rim.
    pub base_spin: f32,
    /// Animations displacing geometry.
    pub warps: &'a [AxisWave<'a>],
    /// Hue animations on an axis other than the ramp's.
    pub hue_axes: &'a [AxisWave<'a>],
    /// Brightness animations on an axis other than the ramp's.
    pub bright_axes: &'a [AxisWave<'a>],
}

/// Everything a frame does per vertex, in one walk of the mesh.
///
/// Fused because the three things it produces all want the same polar
/// coordinates: the ramp coordinate is an angle, a spin is a rotation about the
/// same centre, and a radial animation scales the same radius. Computed
/// separately that was two `atan2` calls a vertex, which a profile put at a
/// third of the frame.
///
/// Position comes out displaced; the ramp coordinate is read from the vertex
/// *before* displacement, so a colour pattern stays glued to the shape while a
/// warp moves it.
pub fn vertex_pass(out: &mut VertexBuffers, mesh: &RefinedMesh, work: VertexWork) {
    let VertexWork {
        field,
        base_spin,
        warps,
        hue_axes,
        bright_axes,
    } = work;

    out.positions.clear();
    out.uvs.clear();
    out.tints.clear();

    // An angle costs an `atan2` and a radius a square root, so decide once
    // whether anything actually asks for them.
    let rotates = base_spin != 0.0 || warps.iter().any(|w| w.target == AnimTarget::Spin);
    let all = || warps.iter().chain(hue_axes).chain(bright_axes);
    let needs_angle = rotates
        || field.phase == ColorPhase::Angle
        || all().any(|w| w.phase == ColorPhase::Angle);
    let needs_radius = rotates
        || field.phase == ColorPhase::Radius
        || warps
            .iter()
            .any(|w| matches!(w.target, AnimTarget::Radial | AnimTarget::AspectRatio))
        || all().any(|w| w.phase == ColorPhase::Radius);
    let tinting = !bright_axes.is_empty();

    for (i, v) in mesh.verts.iter().enumerate() {
        let polar = Polar::of(*v, needs_angle, needs_radius);

        // Geometry.
        let mut radial = 1.0f32;
        let mut turn = base_spin * polar.radius * std::f32::consts::TAU;
        let (mut scale_x, mut scale_y) = (1.0f32, 1.0f32);
        for w in warps {
            let value = w.wave.value_f32(polar.phase(*v, w.phase), i);
            match w.target {
                // Multiplicative, so the deformation is proportional: a
                // waveform around the angle turns a disc into petals.
                AnimTarget::Radial => radial *= 1.0 + value,
                AnimTarget::Spin => turn += value * MAX_SPIN,
                AnimTarget::AspectRatio => {
                    scale_x *= 1.0 + value;
                    scale_y *= 1.0 - value;
                }
                _ => {}
            }
        }
        // A negative radius would turn the shape inside out through the origin
        // rather than collapsing it.
        let radial = radial.max(0.0);
        out.positions.push(if rotates {
            let angle = polar.angle + turn;
            let radius = polar.radius * radial;
            [
                radius * angle.cos() * scale_x,
                radius * angle.sin() * scale_y,
            ]
        } else {
            // Without rotation the angle never changes, so scaling the radius
            // is scaling x and y — no round trip through polar coordinates.
            [v[0] * radial * scale_x, v[1] * radial * scale_y]
        });

        // Colour on the ramp's axis, plus any hue animation on another.
        let mut u = polar.phase(*v, field.phase) * field.cycles;
        for w in hue_axes {
            u += w.wave.value_f32(polar.phase(*v, w.phase), i);
        }
        out.uvs.push([u, 0.5]);

        if tinting {
            let mut brightness = 1.0f32;
            for w in bright_axes {
                let value = w.wave.value_f32(polar.phase(*v, w.phase), i);
                // Only ever darkens, matching what the ramp does with it.
                brightness *= (1.0 + value).clamp(0.0, 1.0);
            }
            out.tints.push([brightness, brightness, brightness, 1.0]);
        }
    }
}

/// A vertex's polar coordinates, computed only where they are wanted.
#[derive(Clone, Copy, Default)]
struct Polar {
    radius: f32,
    angle: f32,
}

impl Polar {
    #[inline]
    fn of(v: [f32; 2], angle: bool, radius: bool) -> Self {
        Self {
            radius: if radius {
                (v[0] * v[0] + v[1] * v[1]).sqrt()
            } else {
                0.0
            },
            angle: if angle {
                crate::fastmath::atan2(v[1], v[0])
            } else {
                0.0
            },
        }
    }

    /// Unit phase along one coordinate, reusing what has already been computed.
    #[inline]
    fn phase(self, v: [f32; 2], phase: ColorPhase) -> f32 {
        match phase {
            ColorPhase::Angle => self.angle / std::f32::consts::TAU + 0.5,
            ColorPhase::Radius => self.radius / std::f32::consts::SQRT_2,
            ColorPhase::LinearX => (v[0] + 1.0) / 2.0,
            ColorPhase::LinearY => (v[1] + 1.0) / 2.0,
        }
    }
}

/// Per-vertex attributes a draw reads, indexed the same way the mesh is.
#[derive(Clone, Copy)]
pub struct Shaded<'a> {
    /// Shape-space positions, displaced if anything warps them.
    pub positions: &'a [[f32; 2]],
    /// Where each vertex looks in the ramp.
    pub uvs: &'a [[f32; 2]],
    /// Multiplier carrying colour animations from a second axis, if any.
    pub tints: Option<&'a [[f32; 4]]>,
}

/// Draw a refined mesh in one flat color.
///
/// Needed because a warped layer cannot take the unrefined fast path even when
/// its color is uniform: the displacement lives on the refined mesh's vertices.
pub fn draw_flat<G: Graphics>(
    mesh: &RefinedMesh,
    positions: &[[f32; 2]],
    color: [f32; 4],
    m: Matrix2d,
    gl: &mut G,
) {
    if mesh.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    gl.tri_list(&DrawState::default(), &color, |f| {
        for chunk in mesh.indices.chunks(stride) {
            pos.clear();
            pos.extend(chunk.iter().map(|&i| project(m, positions[i as usize])));
            f(&pos);
        }
    });
}

/// The single color a uniform or masked layer draws in.
pub fn flat_color(layer: &LayerParams) -> [f32; 4] {
    if layer.mask {
        // A mask paints opaque black, punching a hole in everything beneath.
        [0.0, 0.0, 0.0, 1.0]
    } else {
        hsv_to_rgb(layer.col_center, layer.col_sat, 1.0, layer.level)
    }
}

/// Draw a refined mesh, taking its color from a ramp texture indexed by phase.
///
/// The sampler resolves the waveform per fragment, so the sawtooth's jump lands
/// exactly where it belongs however coarse the mesh is.
pub fn draw_textured<G: Graphics>(
    mesh: &RefinedMesh,
    verts: Shaded,
    period: Option<f32>,
    texture: &G::Texture,
    m: Matrix2d,
    gl: &mut G,
) {
    let Shaded {
        positions,
        uvs,
        tints,
    } = verts;
    if mesh.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    let mut uv = Vec::with_capacity(stride);
    let mut col = Vec::with_capacity(stride);

    // Gather one chunk of triangles into the backend's layout.
    let gather = |chunk: &[u32],
                  pos: &mut Vec<[f32; 2]>,
                  uv: &mut Vec<[f32; 2]>,
                  col: &mut Vec<[f32; 4]>| {
        pos.clear();
        uv.clear();
        col.clear();
        for tri in chunk.chunks(3) {
            let reference = uvs[tri[0] as usize][0];
            for &i in tri {
                let v = uvs[i as usize];
                pos.push(project(m, positions[i as usize]));
                let u = match period {
                    Some(p) => same_branch(reference, v[0], p),
                    None => v[0],
                };
                uv.push([u, v[1]]);
                if let Some(tints) = tints {
                    col.push(tints[i as usize]);
                }
            }
        }
    };

    match tints {
        Some(_) => gl.tri_list_uv_c(&DrawState::default(), texture, |f| {
            for chunk in mesh.indices.chunks(stride) {
                gather(chunk, &mut pos, &mut uv, &mut col);
                f(&pos, &uv, &col);
            }
        }),
        None => gl.tri_list_uv(&DrawState::default(), &[1.0; 4], texture, |f| {
            for chunk in mesh.indices.chunks(stride) {
                gather(chunk, &mut pos, &mut uv, &mut col);
                f(&pos, &uv);
            }
        }),
    }
}

/// Draw one layer's shape.
pub fn draw_layer<G: Graphics>(
    mesh: &ShapeMesh,
    outline: Option<&[[f32; 2]]>,
    layer: &LayerParams,
    transform: Matrix2d,
    gl: &mut G,
) {
    if !layer.enabled {
        return;
    }
    if layer.draw_mode.draws_fill() {
        draw_tris(&mesh.fill, layer, transform, gl);
    }
    if let Some(outline) = outline.filter(|_| layer.draw_mode.draws_outline()) {
        draw_tris(outline, layer, transform, gl);
    }
}

/// Draw a raw triangle list in one flat color.
fn draw_tris<G: Graphics>(
    tris: &[[f32; 2]],
    layer: &LayerParams,
    m: Matrix2d,
    gl: &mut G,
) {
    if tris.is_empty() {
        return;
    }
    // A mask paints opaque black, punching a hole in everything already drawn.
    // This is the `Channel.mask` behavior the real mixer already uses to stack
    // shapes like gobos.
    if layer.mask {
        let mut chunk = Vec::with_capacity(CHUNK);
        gl.tri_list(&DrawState::default(), &[0.0, 0.0, 0.0, 1.0], |f| {
            for tri in tris.chunks(3) {
                if chunk.len() + 3 > CHUNK {
                    f(&chunk);
                    chunk.clear();
                }
                chunk.extend(tri.iter().map(|v| project(m, *v)));
            }
            if !chunk.is_empty() {
                f(&chunk);
            }
        });
        return;
    }

    let color = hsv_to_rgb(layer.col_center, layer.col_sat, 1.0, layer.level);
    let mut chunk = Vec::with_capacity(CHUNK);
    gl.tri_list(&DrawState::default(), &color, |f| {
        for tri in tris.chunks(3) {
            if chunk.len() + 3 > CHUNK {
                f(&chunk);
                chunk.clear();
            }
            chunk.extend(tri.iter().map(|v| project(m, *v)));
        }
        if !chunk.is_empty() {
            f(&chunk);
        }
    });
}

/// Apply a 2D affine matrix to a vertex. Piston's backends take pre-transformed
/// vertices when `tri_list` is called directly.
#[inline]
fn project(m: Matrix2d, v: [f32; 2]) -> [f32; 2] {
    let (x, y) = (f64::from(v[0]), f64::from(v[1]));
    [
        (m[0][0] * x + m[0][1] * y + m[0][2]) as f32,
        (m[1][0] * x + m[1][1] * y + m[1][2]) as f32,
    ]
}
