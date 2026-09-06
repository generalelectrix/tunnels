//! Drawing shape meshes through piston's `Graphics` trait.
//!
//! Generic over the backend so the same code runs on the GL window and on the
//! software rasteriser used for headless contact sheets.

use crate::mesh::RefinedMesh;
use crate::anim::LiveWave;
use crate::params::{AnimTarget, COLOR_SPREAD_SCALE, ColorPhase, LayerParams, PhaseField};
use tunnels_lib::number::UnipolarFloat;
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
    let angle = (layer.rotation + layer.spin_speed * time) * TAU;
    base.trans(layer.x * critical, layer.y * critical)
        .rot_rad(angle)
        .shear(layer.shear_x, layer.shear_y)
        .scale(
            layer.scale_x * critical * 0.5,
            layer.scale_y * critical * 0.5,
        )
}

/// Texture coordinates carrying each vertex's phase.
///
/// The whole point of the split: the mesh holds positions, this holds where
/// each one sits in the color cycle, and the ramp texture turns that into a
/// color at the fragment. A cycle count larger than one falls out for free —
/// the coordinate simply runs past one and the texture repeats.
pub fn phase_uvs(mesh: &RefinedMesh, field: PhaseField) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    phase_uvs_into(&mut out, mesh, field);
    out
}

/// As `phase_uvs`, reusing a buffer so a frame allocates nothing.
pub fn phase_uvs_into(out: &mut Vec<[f32; 2]>, mesh: &RefinedMesh, field: PhaseField) {
    out.clear();
    out.extend(mesh.verts.iter().map(|v| [field.at(*v), 0.5]));
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

/// How far a twist animation can rotate a point at full size: a quarter turn.
const MAX_TWIST: f32 = std::f32::consts::TAU / 4.0;

/// One geometry animation, ready to evaluate.
pub struct GeometryWave<'a> {
    pub target: AnimTarget,
    pub phase: ColorPhase,
    pub wave: &'a LiveWave,
}

/// Whether anything would move a vertex.
pub fn warps_geometry(layer: &LayerParams, waves: &[GeometryWave]) -> bool {
    layer.twist != 0.0 || !waves.is_empty()
}

/// Displace a mesh's vertices under its base twist and geometry animations.
///
/// The mesh itself is untouched — this produces a new position for each vertex,
/// once per frame, the same shape of work as evaluating phase. Nothing here can
/// invalidate the mesh, because the mesh is only ever refined against on-screen
/// size.
///
/// Note what this cannot do: the mesh is not resubdivided, so a warp finer than
/// the triangles carrying it will facet rather than curve. Mesh density is the
/// ceiling on warp frequency, the same way it is on noise frequency.
pub fn warp_verts_into(
    out: &mut Vec<[f32; 2]>,
    mesh: &RefinedMesh,
    base_twist: f32,
    waves: &[GeometryWave],
    audio: UnipolarFloat,
) {
    out.clear();
    out.extend(mesh.verts.iter().enumerate().map(|(i, v)| {
            let (x, y) = (v[0], v[1]);
            let mut radius = (x * x + y * y).sqrt();
            // The base twist grows with radius, so the centre stays put and the
            // rim carries the full turn — a spiral shear rather than a rotation.
            let mut angle = y.atan2(x) + base_twist * radius * std::f32::consts::TAU;
            let (mut scale_x, mut scale_y) = (1.0f32, 1.0f32);

            for w in waves {
                let field = PhaseField {
                    phase: w.phase,
                    cycles: 1.0,
                };
                let value = w.wave.value(f64::from(field.unit_at(*v)), i, audio) as f32;
                match w.target {
                    // Multiplicative, so the deformation is proportional: a
                    // waveform around the angle turns a disc into petals.
                    AnimTarget::Radial => radius *= 1.0 + value,
                    AnimTarget::Twist => angle += value * MAX_TWIST,
                    AnimTarget::AspectRatio => {
                        scale_x *= 1.0 + value;
                        scale_y *= 1.0 - value;
                    }
                    _ => {}
                }
            }

            // A negative radius would turn the shape inside out through the
            // origin rather than collapsing it.
            let radius = radius.max(0.0);
            [
                radius * angle.cos() * scale_x,
                radius * angle.sin() * scale_y,
            ]
        }));
}

/// Draw a refined mesh, taking its color from a ramp texture indexed by phase.
///
/// The sampler resolves the waveform per fragment, so the sawtooth's jump lands
/// exactly where it belongs however coarse the mesh is.
pub fn draw_textured<G: Graphics>(
    mesh: &RefinedMesh,
    positions: &[[f32; 2]],
    uvs: &[[f32; 2]],
    period: Option<f32>,
    texture: &G::Texture,
    m: Matrix2d,
    gl: &mut G,
) {
    if mesh.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut pos = Vec::with_capacity(stride);
    let mut uv = Vec::with_capacity(stride);
    gl.tri_list_uv(&DrawState::default(), &[1.0; 4], texture, |f| {
        for chunk in mesh.indices.chunks(stride) {
            pos.clear();
            uv.clear();
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
                }
            }
            f(&pos, &uv);
        }
    });
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
    if layer.draw_mode.draws_outline() {
        if let Some(outline) = outline {
            draw_tris(outline, layer, transform, gl);
        }
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
