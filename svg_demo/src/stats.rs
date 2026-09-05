//! Cost and sanity reporting for the shape library.
//!
//! Answers two questions: what the gradient subdivision costs, and whether any
//! shape carries geometry that does not belong to it.

use crate::draw::{project_cost, shade_mesh};
use crate::params::{ColorPhase, LayerParams};
use crate::shapes::ShapeMesh;
use graphics::math::{Matrix2d, identity};
use std::time::Instant;

/// Shapes are normalised into [-1, 1]. Anything past this is a stray vertex,
/// not a wide shape.
const OUTSIDE: f32 = 1.02;

/// A triangle whose shortest altitude is below this, relative to its longest
/// edge, is a sliver — long, thin, and usually a tessellation artifact.
const SLIVER_RATIO: f32 = 0.002;

struct Report {
    name: String,
    base: usize,
    shaded: usize,
    micros: u128,
    strays: usize,
    max_extent: f32,
    slivers: usize,
    stroke_strays: usize,
    stroke_extent: f32,
    soft_micros: u128,
}

/// Twice the area of a triangle.
fn double_area(t: &[[f32; 2]]) -> f32 {
    ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])).abs()
}

fn longest_edge(t: &[[f32; 2]]) -> f32 {
    let d = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    d(t[0], t[1]).max(d(t[1], t[2])).max(d(t[2], t[0]))
}

pub fn report(shapes: &[ShapeMesh]) {
    // A gradient busy enough to be representative: several cycles, so several
    // discontinuities, which is what drives the subdivision.
    let layer = LayerParams {
        enabled: true,
        color_phase: ColorPhase::Angle,
        col_center: 0.45,
        col_width: 0.8,
        col_spread: 0.35,
        col_sat: 0.9,
        ..Default::default()
    };
    let m: Matrix2d = identity();

    let mut reports = Vec::new();
    for shape in shapes {
        let start = Instant::now();
        let (pos, _) = shade_mesh(&shape.fill, &layer);
        let micros = start.elapsed().as_micros();

        // Per-frame cost once the subdivision is cached: just the transform.
        let start = Instant::now();
        let mut sink = 0f32;
        for _ in 0..8 {
            sink += pos.iter().map(|v| project_cost(m, *v)).sum::<f32>();
        }
        std::hint::black_box(sink);
        let soft_micros = start.elapsed().as_micros() / 8;

        let mut strays = 0;
        let mut max_extent = 0f32;
        for v in &shape.fill {
            let e = v[0].abs().max(v[1].abs());
            max_extent = max_extent.max(e);
            if e > OUTSIDE || !v[0].is_finite() || !v[1].is_finite() {
                strays += 1;
            }
        }
        // Outline mode runs the stroke tessellator, which is the likelier
        // source of stray geometry: a closed path whose final point coincides
        // with its first leaves a zero-length segment, and round joins on that
        // can produce garbage.
        let stroke = shape.stroke(0.03);
        let mut stroke_strays = 0;
        let mut stroke_extent = 0f32;
        for v in &stroke {
            let e = v[0].abs().max(v[1].abs());
            stroke_extent = stroke_extent.max(e);
            if e > OUTSIDE + 0.05 || !v[0].is_finite() || !v[1].is_finite() {
                stroke_strays += 1;
            }
        }

        let slivers = shape
            .fill
            .chunks(3)
            .filter(|t| t.len() == 3)
            .filter(|t| {
                let longest = longest_edge(t);
                longest > 0.0 && double_area(t) / longest < SLIVER_RATIO * longest
            })
            .count();

        reports.push(Report {
            name: shape.name.clone(),
            base: shape.fill.len() / 3,
            shaded: pos.len() / 3,
            micros,
            strays,
            max_extent,
            slivers,
            stroke_strays,
            stroke_extent,
            soft_micros,
        });
    }

    println!("\n=== gradient subdivision cost ===");
    println!("{:<46} {:>7} {:>9} {:>7} {:>6}", "shape", "tris", "subdiv", "ratio", "us");
    reports.sort_by_key(|r| std::cmp::Reverse(r.micros));
    for r in reports.iter().take(12) {
        println!(
            "{:<46} {:>7} {:>9} {:>6.1}x {:>6}",
            r.name,
            r.base,
            r.shaded,
            r.shaded as f64 / r.base.max(1) as f64,
            r.micros
        );
    }
    let total_us: u128 = reports.iter().map(|r| r.micros).sum();
    let worst = reports.iter().map(|r| r.micros).max().unwrap_or(0);
    let median = {
        let mut v: Vec<u128> = reports.iter().map(|r| r.micros).collect();
        v.sort_unstable();
        v[v.len() / 2]
    };
    println!(
        "\n{} shapes: median {median}us, worst {worst}us, whole library {}us",
        reports.len(),
        total_us
    );
    println!(
        "three worst-case layers per frame: {:.2}ms of a 16.7ms budget at 60Hz",
        (worst * 3) as f64 / 1000.0
    );

    println!("\n=== per-frame cost once subdivision is cached ===");
    let soft_total: u128 = reports.iter().map(|r| r.soft_micros).sum();
    let soft_worst = reports.iter().map(|r| r.soft_micros).max().unwrap_or(0);
    let soft_median = {
        let mut v: Vec<u128> = reports.iter().map(|r| r.soft_micros).collect();
        v.sort_unstable();
        v[v.len() / 2]
    };
    println!("transform only: median {soft_median}us, worst {soft_worst}us, library {soft_total}us");
    println!(
        "three worst-case layers per frame: {:.2}ms of a 16.7ms budget at 60Hz",
        (soft_worst * 3) as f64 / 1000.0
    );

    println!("\n=== stray geometry ===");
    let mut flagged = 0;
    for r in reports.iter() {
        if r.strays > 0 || r.stroke_strays > 0 {
            flagged += 1;
            println!(
                "{:<46} fill {:>4} (max |v| {:>8.3})  stroke {:>5} (max |v| {:>8.3})",
                r.name, r.strays, r.max_extent, r.stroke_strays, r.stroke_extent
            );
        }
    }
    if flagged == 0 {
        println!("none: every fill and stroke vertex is finite and inside the unit box");
    }

    let sliver_total: usize = reports.iter().map(|r| r.slivers).sum();
    let worst_slivers = reports.iter().max_by_key(|r| r.slivers);
    println!("\n=== slivers (long thin fill triangles from the tessellator) ===");
    println!("{sliver_total} across the library");
    if let Some(r) = worst_slivers {
        println!("worst: {} with {} of {} triangles", r.name, r.slivers, r.base);
    }
}
