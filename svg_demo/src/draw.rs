//! Drawing shape meshes through piston's `Graphics` trait.
//!
//! Generic over the backend so the same code runs on the GL window and on the
//! software rasteriser used for headless contact sheets.

use crate::params::{ColorPhase, LayerParams};
use crate::shapes::ShapeMesh;
use graphics::draw_state::DrawState;
use graphics::math::Matrix2d;
use graphics::{Graphics, Transformed};
use std::f64::consts::TAU;

/// Vertices per chunk handed to the backend. `BACK_END_MAX_VERTEX_COUNT` is
/// 1023, which divides evenly into triangles.
const CHUNK: usize = 1023;

/// Largest color difference, per channel, tolerated across a triangle before it
/// is split.
///
/// Per-vertex color interpolates linearly, so a triangle only looks right where
/// the color varies close to linearly across it. The sawtooth `Tunnel` colors
/// with is discontinuous once per cycle, and a triangle straddling that jump
/// renders it as a ramp across the whole triangle — which reads as a ragged
/// edge zigzagging along the mesh rather than a clean boundary.
const COLOR_STEP: f32 = 0.1;

/// Coarse cap on triangle size, in normalised shape units.
///
/// The color test alone can be fooled by a triangle whose corners happen to
/// land on the same color while its interior sweeps a whole cycle.
const SAFETY_EDGE: f32 = 0.25;

/// Ceiling on subdivision recursion.
///
/// Triangles straddling the sawtooth discontinuity never satisfy the color test
/// however small they get, so they always recurse to this depth. Only the ones
/// touching the discontinuity do, and their count grows with the length of that
/// boundary rather than with area, so this stays cheap.
const MAX_DEPTH: u32 = 8;

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

/// How many hue cycles a full turn of `col_spread` buys. Matches
/// `COLOR_SPREAD_SCALE` in `tunnels/src/tunnel.rs`.
const COLOR_SPREAD_SCALE: f64 = 16.0;

/// The rising sawtooth `Tunnel` colors with, on [0, 1) returning [-1, 1).
///
/// Ported from `waveforms::sawtooth` with smoothing off, pulse off, and a full
/// duty cycle — the settings `Tunnel::render` passes when it builds a hue.
fn sawtooth(phase: f64) -> f64 {
    let phase = phase.rem_euclid(1.0);
    if phase < 0.5 { 2.0 * phase } else { 2.0 * (phase - 1.0) }
}

/// Where a point sits along the coordinate driving the color sawtooth,
/// as a fraction in [0, 1).
///
/// This stands in for a tunnel segment's `rel_angle`. Shape space is the
/// normalised unit box, so the gradient rides with the figure rather than being
/// pinned to the screen.
fn color_phase(p: [f32; 2], layer: &LayerParams) -> f64 {
    let (x, y) = (f64::from(p[0]), f64::from(p[1]));
    match layer.color_phase {
        ColorPhase::Angle => y.atan2(x) / TAU,
        // Shapes are normalised into a unit box, so the far corner is at
        // sqrt(2). Dividing by that keeps a full sweep inside one cycle.
        ColorPhase::Radius => (x * x + y * y).sqrt() / std::f64::consts::SQRT_2,
        ColorPhase::LinearX => (x + 1.0) / 2.0,
        ColorPhase::LinearY => (y + 1.0) / 2.0,
    }
}

/// The hue at a point in shape space.
///
/// This is `Tunnel::render`'s hue expression with the shape's own coordinate
/// standing in for a segment's position around the ring, so a look dialled in
/// here maps onto the same four knobs on the real control surface.
fn hue_at(p: [f32; 2], layer: &LayerParams) -> f64 {
    let cycles = (COLOR_SPREAD_SCALE * layer.col_spread).floor();
    let phase = color_phase(p, layer) * cycles;
    layer.col_center + 0.5 * layer.col_width * sawtooth(phase)
}

/// Whether every vertex of this layer resolves to the same color, letting the
/// cheaper flat-color path handle it.
fn is_flat(layer: &LayerParams) -> bool {
    layer.col_width == 0.0 || (COLOR_SPREAD_SCALE * layer.col_spread).floor() == 0.0
}

/// The color of a point in shape space under this layer.
fn shade(v: [f32; 2], layer: &LayerParams) -> [f32; 4] {
    hsv_to_rgb(hue_at(v, layer), layer.col_sat, 1.0, layer.level)
}

/// Split a triangle until linear color interpolation across it is faithful,
/// appending screen-space positions and their colors.
#[expect(clippy::too_many_arguments)]
fn shade_triangle(
    tri: [[f32; 2]; 3],
    colors: [[f32; 4]; 3],
    depth: u32,
    layer: &LayerParams,
    m: Matrix2d,
    pos: &mut Vec<[f32; 2]>,
    col: &mut Vec<[f32; 4]>,
) {
    let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let longest = dist(tri[0], tri[1])
        .max(dist(tri[1], tri[2]))
        .max(dist(tri[2], tri[0]));
    // Compare colors rather than hues: hue wraps, so two nearby hues can be far
    // apart numerically, and it is the interpolated color that has to be right.
    let spread = (0..3)
        .flat_map(|i| (0..4).map(move |ch| (i, ch)))
        .map(|(i, ch)| {
            let v = colors[i][ch];
            (v - colors[(i + 1) % 3][ch]).abs()
        })
        .fold(0.0f32, f32::max);

    if depth >= MAX_DEPTH || (spread <= COLOR_STEP && longest <= SAFETY_EDGE) {
        for i in 0..3 {
            pos.push(project(m, tri[i]));
            col.push(colors[i]);
        }
        return;
    }

    // Midpoint split. The new vertices sit exactly on the parent edges, so a
    // neighbour that stops subdividing sooner leaves no crack — only a colour
    // difference smaller than the threshold that stopped it.
    let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (a, b, c) = (tri[0], tri[1], tri[2]);
    let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
    let (cab, cbc, cca) = (shade(ab, layer), shade(bc, layer), shade(ca, layer));
    let children = [
        ([a, ab, ca], [colors[0], cab, cca]),
        ([ab, b, bc], [cab, colors[1], cbc]),
        ([ca, bc, c], [cca, cbc, colors[2]]),
        ([ab, bc, ca], [cab, cbc, cca]),
    ];
    for (t, cs) in children {
        shade_triangle(t, cs, depth + 1, layer, m, pos, col);
    }
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

    // A uniform layer takes the cheaper single-color path; gradients need
    // per-vertex colors, the only thing in the project exercising tri_list_c.
    if is_flat(layer) {
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
        return;
    }

    // Subdivide first, then hand the result over in chunks — a triangle can
    // expand into many, so the split cannot happen inside the chunking loop.
    let mut pos = Vec::new();
    let mut col = Vec::new();
    for tri in tris.chunks(3) {
        let [a, b, c] = tri else { continue };
        let corners = [*a, *b, *c];
        let colors = [shade(*a, layer), shade(*b, layer), shade(*c, layer)];
        shade_triangle(corners, colors, 0, layer, m, &mut pos, &mut col);
    }
    gl.tri_list_c(&DrawState::default(), |f| {
        for (p, c) in pos.chunks(CHUNK / 3 * 3).zip(col.chunks(CHUNK / 3 * 3)) {
            f(p, c);
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
