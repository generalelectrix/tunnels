//! Cost and sanity reporting for the shape library.
//!
//! Answers three questions: what each mesh density costs to build and to hold,
//! what a frame costs once the meshes exist, and whether any shape carries
//! geometry that does not belong to it.

use crate::anim::{LiveWave, WaveformKind};
use crate::draw::{AxisWave, VertexBuffers, VertexWork, vertex_pass};
use crate::params::{AnimTarget, WaveParams};

use crate::mesh::{self, Level, refine};
use crate::ramp;
use crate::params::{ColorPhase, LayerParams, PhaseField};
use crate::shapes::ShapeMesh;
use std::time::Instant;

/// Shapes are normalised into [-1, 1]. Anything past this is a stray vertex,
/// not a wide shape.
const OUTSIDE: f32 = 1.02;

/// A triangle whose area is negligible against its longest edge is a sliver —
/// long, thin, and usually a tessellation artifact.
const SLIVER_RATIO: f32 = 0.002;

/// Iterations each per-vertex measurement is averaged over.
const REPEATS: u128 = 16;

/// The shape with the most source triangles, which refines into the most
/// vertices and so is the worst case for anything measured per vertex.
fn heaviest(shapes: &[ShapeMesh]) -> usize {
    (0..shapes.len())
        .max_by_key(|&i| shapes[i].fill.len())
        .unwrap_or(0)
}

fn double_area(t: &[[f32; 2]]) -> f32 {
    ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])).abs()
}

fn longest_edge(t: &[[f32; 2]]) -> f32 {
    let d = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    d(t[0], t[1]).max(d(t[1], t[2])).max(d(t[2], t[0]))
}

pub fn report(shapes: &[ShapeMesh]) {
    // A gradient busy enough to be representative: several cycles, so the
    // color moves fast across the whole shape.
    let layer = LayerParams {
        enabled: true,
        color_phase: ColorPhase::Angle,
        col_center: 0.45,
        col_width: 0.8,
        col_spread: 0.35,
        col_sat: 0.9,
        ..Default::default()
    };

    println!("\n=== mesh levels ===");
    println!(
        "{:>7} {:>9} {:>12} {:>12} {:>10} {:>10}",
        "edge", "build ms", "tris (med)", "tris (max)", "verts (max)", "MB all"
    );
    for level in Level::all() {
        let target = level.target_edge();
        let start = Instant::now();
        let meshes: Vec<_> = shapes.iter().map(|s| refine(&s.fill, target)).collect();
        let build_ms = start.elapsed().as_secs_f64() * 1000.0;

        let mut tris: Vec<usize> = meshes.iter().map(|m| m.triangle_count()).collect();
        tris.sort_unstable();
        let max_verts = meshes.iter().map(|m| m.verts.len()).max().unwrap_or(0);
        // Position pairs plus indices, which is what a mesh actually holds.
        let bytes: usize = meshes
            .iter()
            .map(|m| m.verts.len() * 8 + m.indices.len() * 4)
            .sum();
        println!(
            "{:>7.4} {:>9.1} {:>12} {:>12} {:>10} {:>10.1}",
            target,
            build_ms,
            tris[tris.len() / 2],
            tris[tris.len() - 1],
            max_verts,
            bytes as f64 / 1e6
        );
    }

    // Per-frame work at the density a full-screen shape on a 1080-line
    // projector asks for.
    let level = Level::for_scale(1.0, 1080.0, mesh::DEFAULT_TARGET_PX);
    println!(
        "\n=== per frame at level {:?} (edge {:.4}) ===",
        level,
        level.target_edge()
    );
    let mut color_us = Vec::new();
    for shape in shapes {
        let mesh = refine(&shape.fill, level.target_edge());
        let mut buf = VertexBuffers::default();
        let start = Instant::now();
        for _ in 0..8 {
            vertex_pass(
                &mut buf,
                &mesh,
                VertexWork {
                    field: PhaseField::of(&layer),
                    base_spin: 0.0,
                    warps: &[],
                    hue_axes: &[],
                    bright_axes: &[],
                },
            );
        }
        color_us.push((start.elapsed().as_micros() / 8, mesh.verts.len(), &shape.name));
    }
    color_us.sort_unstable();
    let median = color_us[color_us.len() / 2].0;
    let (worst, worst_verts, worst_name) = color_us[color_us.len() - 1];
    println!("phase evaluation: median {median}us, worst {worst}us ({worst_verts} verts, {worst_name})");
    println!(
        "three worst-case layers: {:.2}ms of a 16.7ms budget at 60Hz",
        (worst * 3) as f64 / 1000.0
    );

    // The profiler attributes about a millisecond a frame to rebuilding ramps
    // when hue is sweeping. Split that into the CPU half measurable here and
    // the GL upload, which is not.
    // Triangle count goes as the inverse square of the target, and submitting
    // vertices is the largest per-frame cost, so this is the dial to reach for
    // on slow hardware.
    println!("\n=== triangles at 1080p full screen, by target triangle size ===");
    println!("{:>10} {:>8} {:>14} {:>14}", "target px", "edge", "median tris", "max tris");
    for px in [7.0, 10.0, 14.0, 20.0, 28.0, 40.0] {
        let lvl = Level::for_scale(1.0, 1080.0, px);
        let mut tris: Vec<usize> = shapes
            .iter()
            .map(|s| refine(&s.fill, lvl.target_edge()).triangle_count())
            .collect();
        tris.sort_unstable();
        println!(
            "{px:>10.0} {:>8.4} {:>14} {:>14}",
            lvl.target_edge(),
            tris[tris.len() / 2],
            tris[tris.len() - 1]
        );
    }

    println!("\n=== ramp build (CPU only; excludes the texture upload) ===");
    let mut probe = layer.clone();
    let start = Instant::now();
    for i in 0..200 {
        probe.col_center = f64::from(i) / 200.0;
        std::hint::black_box(ramp::build(&probe));
    }
    let per = start.elapsed().as_nanos() / 200;
    println!(
        "{} texels: {:.0}us per rebuild ({:.1}ns per texel)",
        ramp::RAMP_TEXELS,
        per as f64 / 1000.0,
        per as f64 / f64::from(ramp::RAMP_TEXELS)
    );

    // Animations on the layer's own colour axis are baked into the ramp, a
    // fixed thousand evaluations however many there are. Anything on a second
    // axis, and every geometry animation, is evaluated once per vertex per
    // frame instead — so its cost scales with the mesh.
    println!("\n=== per-vertex pass, at the density above ===");
    let worst = heaviest(shapes);
    let mesh = refine(&shapes[worst].fill, level.target_edge());
    let verts = mesh.verts.len();
    println!("{verts} vertices ({})", shapes[worst].name);

    let params = |waveform: WaveformKind| WaveParams {
        waveform,
        n_periods: 4,
        size: 0.6,
        ..Default::default()
    };
    let sine = LiveWave::new(&params(WaveformKind::Sine));
    let noise = LiveWave::new(&params(WaveformKind::Noise));
    let axis = |target, wave| AxisWave {
        target,
        phase: ColorPhase::Radius,
        wave,
    };

    let field = PhaseField::of(&layer);
    let mut buffers = VertexBuffers::default();

    let mut measure = |label: &str,
                       warps: &[AxisWave],
                       hue_axes: &[AxisWave],
                       bright_axes: &[AxisWave]| {
        let start = Instant::now();
        for _ in 0..REPEATS {
            vertex_pass(
                &mut buffers,
                &mesh,
                VertexWork {
                    field,
                    base_spin: 0.0,
                    warps,
                    hue_axes,
                    bright_axes,
                },
            );
        }
        let us = start.elapsed().as_micros() / REPEATS;
        println!(
            "{label:<42} {us:>5}us  ({:.0}ns per vertex)",
            us as f64 * 1000.0 / verts as f64
        );
    };

    measure("nothing animated (baseline)", &[], &[], &[]);
    measure("one colour animation, 2nd axis", &[], &[axis(AnimTarget::Hue, &sine)], &[]);
    measure(
        "two colour animations, 2nd axis",
        &[],
        &[axis(AnimTarget::Hue, &sine), axis(AnimTarget::Hue, &sine)],
        &[],
    );
    measure("one noise colour animation", &[], &[axis(AnimTarget::Hue, &noise)], &[]);
    measure("one geometry animation (radial)", &[axis(AnimTarget::Radial, &sine)], &[], &[]);
    measure("one geometry animation (spin)", &[axis(AnimTarget::Spin, &sine)], &[], &[]);

    println!("\n=== stray geometry ===");
    let mut flagged = 0;
    for shape in shapes {
        let stroke = shape.stroke(0.03);
        let bad = |vs: &[[f32; 2]], limit: f32| {
            vs.iter()
                .filter(|v| {
                    v[0].abs().max(v[1].abs()) > limit || !v[0].is_finite() || !v[1].is_finite()
                })
                .count()
        };
        let (f, s) = (bad(&shape.fill, OUTSIDE), bad(&stroke, OUTSIDE + 0.05));
        if f > 0 || s > 0 {
            flagged += 1;
            println!("{:<46} fill {f:>4}  stroke {s:>5}", shape.name);
        }
    }
    if flagged == 0 {
        println!("none: every fill and stroke vertex is finite and inside the unit box");
    }

    let slivers: usize = shapes
        .iter()
        .map(|s| {
            s.fill
                .chunks(3)
                .filter(|t| t.len() == 3)
                .filter(|t| {
                    let l = longest_edge(t);
                    l > 0.0 && double_area(t) / l < SLIVER_RATIO * l
                })
                .count()
        })
        .sum();
    println!("\nslivers in source tessellation: {slivers} across the library");
}
