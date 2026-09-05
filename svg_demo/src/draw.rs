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
/// Kept tight because a triangle that stops subdividing next to one that did
/// not shares a vertex whose color is computed on one side and interpolated on
/// the other. That mismatch is bounded by this, and at a coarser setting it
/// shows up as a thin wedge of slightly wrong color along the seam.
const COLOR_STEP: f32 = 0.04;

/// Coarse cap on triangle size, in normalised shape units.
///
/// The color test alone can be fooled by a triangle whose corners happen to
/// land on the same color while its interior sweeps a whole cycle.
const SAFETY_EDGE: f32 = 0.25;

/// Floor on triangle size, in normalised shape units.
///
/// This is what bounds the work. The sawtooth is discontinuous, so a triangle
/// straddling the jump can never satisfy the color test however small it gets —
/// without a floor it recurses until the depth limit stops it, and a single
/// triangle can explode into tens of thousands. With a floor, the total is
/// bounded by the shape's area divided by the floor squared, which is a few
/// thousand triangles no matter how many cycles the gradient carries.
///
/// The cost of the floor is that the discontinuity renders as a ramp one
/// triangle wide rather than a hard edge. That reads as a soft transition,
/// which is fine; what looked broken before was the boundary zigzagging along
/// the mesh, and the color test still fixes that.
const MIN_EDGE: f32 = 0.012;

/// Backstop on recursion. `MIN_EDGE` normally stops the descent first.
///
/// Bisection halves a triangle's area per level rather than quartering it, so
/// this sits about twice as deep as it would for a four-way split.
const MAX_DEPTH: u32 = 22;

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
pub fn is_uniform(layer: &LayerParams) -> bool {
    layer.col_width == 0.0 || (COLOR_SPREAD_SCALE * layer.col_spread).floor() == 0.0
}

/// The color of a point in shape space under this layer.
fn shade(v: [f32; 2], layer: &LayerParams) -> [f32; 4] {
    hsv_to_rgb(hue_at(v, layer), layer.col_sat, 1.0, layer.level)
}

/// Split a triangle until linear color interpolation across it is faithful,
/// appending shape-space positions and their colors.
fn shade_triangle(
    tri: [[f32; 2]; 3],
    colors: [[f32; 4]; 3],
    depth: u32,
    layer: &LayerParams,
    pos: &mut Vec<[f32; 2]>,
    col: &mut Vec<[f32; 4]>,
) {
    let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    // Which edge is longest decides where to cut. Splitting only that one
    // refines a long thin triangle along its length instead of shattering it in
    // both directions, which matters because a tessellator emits plenty of them
    // — a slat is two units long and a twentieth of a unit tall.
    let lengths = [
        dist(tri[0], tri[1]),
        dist(tri[1], tri[2]),
        dist(tri[2], tri[0]),
    ];
    let cut = (0..3).fold(0, |best, i| if lengths[i] > lengths[best] { i } else { best });
    let longest = lengths[cut];
    // Compare colors rather than hues: hue wraps, so two nearby hues can be far
    // apart numerically, and it is the interpolated color that has to be right.
    let spread = (0..3)
        .flat_map(|i| (0..4).map(move |ch| (i, ch)))
        .map(|(i, ch)| {
            let v = colors[i][ch];
            (v - colors[(i + 1) % 3][ch]).abs()
        })
        .fold(0.0f32, f32::max);

    if depth >= MAX_DEPTH
        || longest <= MIN_EDGE
        || (spread <= COLOR_STEP && longest <= SAFETY_EDGE)
    {
        pos.extend_from_slice(&tri);
        col.extend_from_slice(&colors);
        return;
    }

    // Bisect the longest edge. The new vertex sits exactly on it, so a
    // neighbour that stops subdividing sooner leaves no crack — only a color
    // difference smaller than the threshold that stopped it.
    let (i, j, k) = match cut {
        0 => (0, 1, 2),
        1 => (1, 2, 0),
        _ => (2, 0, 1),
    };
    let mid = [
        (tri[i][0] + tri[j][0]) / 2.0,
        (tri[i][1] + tri[j][1]) / 2.0,
    ];
    let cmid = shade(mid, layer);
    for (t, cs) in [
        ([tri[i], mid, tri[k]], [colors[i], cmid, colors[k]]),
        ([mid, tri[j], tri[k]], [cmid, colors[j], colors[k]]),
    ] {
        shade_triangle(t, cs, depth + 1, layer, pos, col);
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
    if is_uniform(layer) {
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

    let (pos, col) = shade_mesh(tris, layer);
    draw_shaded(&pos, &col, m, gl);
}

/// Draw an already-subdivided mesh, projecting it into screen space.
///
/// Kept separate from `shade_mesh` so a caller can subdivide once and redraw
/// under a moving transform without paying for the subdivision again.
pub fn draw_shaded<G: Graphics>(
    pos: &[[f32; 2]],
    col: &[[f32; 4]],
    m: Matrix2d,
    gl: &mut G,
) {
    if pos.is_empty() {
        return;
    }
    let stride = CHUNK / 3 * 3;
    let mut screen = Vec::with_capacity(stride);
    gl.tri_list_c(&DrawState::default(), |f| {
        for (p, c) in pos.chunks(stride).zip(col.chunks(stride)) {
            screen.clear();
            screen.extend(p.iter().map(|v| project(m, *v)));
            f(&screen, c);
        }
    });
}

/// Subdivide and shade a triangle list, returning shape-space positions and
/// their per-vertex colors.
pub fn shade_mesh(tris: &[[f32; 2]], layer: &LayerParams) -> (Vec<[f32; 2]>, Vec<[f32; 4]>) {
    let mut pos = Vec::new();
    let mut col = Vec::new();
    for tri in tris.chunks(3) {
        let [a, b, c] = tri else { continue };
        let colors = [shade(*a, layer), shade(*b, layer), shade(*c, layer)];
        shade_triangle([*a, *b, *c], colors, 0, layer, &mut pos, &mut col);
    }
    (pos, col)
}

/// The projection step alone, exposed so its per-frame cost can be measured.
pub fn project_cost(m: Matrix2d, v: [f32; 2]) -> f32 {
    let p = project(m, v);
    p[0] + p[1]
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
