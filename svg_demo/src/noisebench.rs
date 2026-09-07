//! Where the time goes in a `Waveform::Noise` evaluation.
//!
//! Walks the same sweep the per-vertex pass walks — an arbitrary spatial phase
//! per vertex and the vertex ordinal as the second noise axis — through a
//! series of loops that differ by one ingredient each, so subtracting two
//! timings names the cost of what was taken out.

use noise::{NoiseFn, Simplex};
use std::hint::black_box;
use std::time::Instant;
use tunnels_lib::number::{Phase, UnipolarFloat};
use tunnels_model::animation::{
    Animation, ControlMessage, EmitStateChange, PreparedAnimation, StateChange, Waveform,
};
use tunnels_model::clock_bank::StaticClockBank;

/// The vertex count of the heaviest shape at projector density.
const N: usize = 43_000;

/// Enough repeats that a run is tens of milliseconds.
const REPEATS: usize = 200;

struct Discard;
impl EmitStateChange for Discard {
    fn emit_animation_state_change(&mut self, _: StateChange) {}
}

fn prepared(waveform: Waveform) -> PreparedAnimation {
    let mut animation = Animation::default();
    let mut sink = Discard;
    let mut set = |sc: StateChange| animation.control(ControlMessage::Set(sc), &mut sink);
    set(StateChange::Waveform(waveform));
    set(StateChange::NPeriods(4));
    set(StateChange::Size(UnipolarFloat::new(0.6)));
    // Settle the smoothing smoother, as the demo does.
    animation.update_state(std::time::Duration::from_secs(1), UnipolarFloat::ZERO);
    animation.prepare(&StaticClockBank::default(), UnipolarFloat::ZERO)
}

/// Spatial phases standing in for a mesh's vertex angles: neither sorted nor
/// evenly spaced, so the noise cell a vertex lands in is as unpredictable as
/// the real one.
fn phases() -> Vec<f64> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..N)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

fn time(label: &str, f: impl Fn() -> f64) {
    // Warm the caches and the branch predictors before the clock starts.
    black_box(f());
    let start = Instant::now();
    let mut acc = 0.0;
    for _ in 0..REPEATS {
        acc += f();
    }
    let elapsed = start.elapsed().as_secs_f64();
    black_box(acc);
    println!(
        "{label:<46} {:>7.2}ns per sample",
        elapsed * 1e9 / (REPEATS * N) as f64
    );
}

// --- the noise arm, rebuilt so its kernel can be swapped -------------------

/// The constants `PreparedAnimation` resolves once a frame, for a default
/// animation with four periods.
struct Arm {
    n_periods: f64,
    phase_temporal: f64,
    smoothing: f64,
    ticks: f64,
    scale: f64,
}

impl Arm {
    /// The arm's arithmetic up to the noise call, returning the point it would
    /// sample. Duty cycle is one here, as it is in the measured case, so the
    /// early return never fires and is left out.
    #[inline(always)]
    fn point(&self, spatial_phase_offset: f64, offset_index: usize) -> [f64; 2] {
        let spatial_phase = spatial_phase_offset * self.n_periods;
        let x_offset = self.ticks + spatial_phase + self.phase_temporal;
        let y_offset = (1.0 - self.smoothing) * offset_index as f64;
        [x_offset, y_offset]
    }
}

/// A kernel with the shape of a noise function and none of its work, to price
/// everything wrapped around the real one.
#[inline(always)]
fn stub(p: [f64; 2]) -> f64 {
    p[0] * 0.5 + p[1] * 0.25
}

// --- a copy of the crate's 2D simplex, to vary one thing at a time ---------

const SKEW: f64 = 0.366_025_403_784_438_6; // (sqrt(3) - 1) / 2
const UNSKEW: f64 = 0.211_324_865_405_187_1; // (1 - 1/sqrt(3)) / 2

#[inline(always)]
fn grad2(index: usize) -> [f64; 2] {
    const D: f64 = std::f64::consts::FRAC_1_SQRT_2;
    match index % 8 {
        0 => [1.0, 0.0],
        1 => [-1.0, 0.0],
        2 => [0.0, 1.0],
        3 => [0.0, -1.0],
        4 => [D, D],
        5 => [-D, D],
        6 => [D, -D],
        _ => [-D, -D],
    }
}

#[inline(always)]
fn surflet(g: [f64; 2], dx: f64, dy: f64) -> f64 {
    let t = 1.0 - (dx * dx + dy * dy) * 2.0;
    if t > 0.0 {
        let t2 = t * t;
        (2.0 * t2 + t2 * t2) * (dx * g[0] + dy * g[1])
    } else {
        0.0
    }
}

/// The crate's algorithm with the analytic derivative left out, so its cost —
/// if the optimiser is not already dropping it — shows up as a difference.
#[inline(always)]
fn simplex_2d_value(point: [f64; 2], table: &[u8; 256]) -> f64 {
    let skew = (point[0] + point[1]) * SKEW;
    let (i, j) = ((point[0] + skew).floor(), (point[1] + skew).floor());
    let unskew = (i + j) * UNSKEW;
    let (dx0, dy0) = (point[0] - (i - unskew), point[1] - (j - unskew));

    let (oi, oj) = if dx0 > dy0 { (1.0, 0.0) } else { (0.0, 1.0) };
    let (dx1, dy1) = (dx0 - oi + UNSKEW, dy0 - oj + UNSKEW);
    let (dx2, dy2) = (dx0 - 1.0 + 2.0 * UNSKEW, dy0 - 1.0 + 2.0 * UNSKEW);

    let (ci, cj) = (i as isize, j as isize);
    let hash = |a: isize, b: isize| -> usize {
        let idx = table[(a & 0xff) as usize] as usize ^ (b & 0xff) as usize;
        table[idx] as usize
    };
    let g0 = grad2(hash(ci, cj));
    let g1 = grad2(hash(ci + oi as isize, cj + oj as isize));
    let g2 = grad2(hash(ci + 1, cj + 1));

    surflet(g0, dx0, dy0) + surflet(g1, dx1, dy1) + surflet(g2, dx2, dy2)
}

/// The same, in single precision.
#[inline(always)]
fn simplex_2d_value_f32(point: [f32; 2], table: &[u8; 256]) -> f32 {
    const SKEW: f32 = 0.366_025_4;
    const UNSKEW: f32 = 0.211_324_87;
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;

    #[inline(always)]
    fn grad(index: usize) -> [f32; 2] {
        match index % 8 {
            0 => [1.0, 0.0],
            1 => [-1.0, 0.0],
            2 => [0.0, 1.0],
            3 => [0.0, -1.0],
            4 => [D, D],
            5 => [-D, D],
            6 => [D, -D],
            _ => [-D, -D],
        }
    }
    #[inline(always)]
    fn surflet(g: [f32; 2], dx: f32, dy: f32) -> f32 {
        let t = 1.0 - (dx * dx + dy * dy) * 2.0;
        if t > 0.0 {
            let t2 = t * t;
            (2.0 * t2 + t2 * t2) * (dx * g[0] + dy * g[1])
        } else {
            0.0
        }
    }

    let skew = (point[0] + point[1]) * SKEW;
    let (i, j) = ((point[0] + skew).floor(), (point[1] + skew).floor());
    let unskew = (i + j) * UNSKEW;
    let (dx0, dy0) = (point[0] - (i - unskew), point[1] - (j - unskew));

    let (oi, oj) = if dx0 > dy0 { (1.0, 0.0) } else { (0.0, 1.0) };
    let (dx1, dy1) = (dx0 - oi + UNSKEW, dy0 - oj + UNSKEW);
    let (dx2, dy2) = (dx0 - 1.0 + 2.0 * UNSKEW, dy0 - 1.0 + 2.0 * UNSKEW);

    let (ci, cj) = (i as isize, j as isize);
    let hash = |a: isize, b: isize| -> usize {
        let idx = table[(a & 0xff) as usize] as usize ^ (b & 0xff) as usize;
        table[idx] as usize
    };
    let g0 = grad(hash(ci, cj));
    let g1 = grad(hash(ci + oi as isize, cj + oj as isize));
    let g2 = grad(hash(ci + 1, cj + 1));

    surflet(g0, dx0, dy0) + surflet(g1, dx1, dy1) + surflet(g2, dx2, dy2)
}

/// The same again, with the permutation table replaced by an integer mix, to
/// price the table's two dependent loads per corner.
#[inline(always)]
fn simplex_2d_value_mixhash(point: [f64; 2]) -> f64 {
    let skew = (point[0] + point[1]) * SKEW;
    let (i, j) = ((point[0] + skew).floor(), (point[1] + skew).floor());
    let unskew = (i + j) * UNSKEW;
    let (dx0, dy0) = (point[0] - (i - unskew), point[1] - (j - unskew));

    let (oi, oj) = if dx0 > dy0 { (1.0, 0.0) } else { (0.0, 1.0) };
    let (dx1, dy1) = (dx0 - oi + UNSKEW, dy0 - oj + UNSKEW);
    let (dx2, dy2) = (dx0 - 1.0 + 2.0 * UNSKEW, dy0 - 1.0 + 2.0 * UNSKEW);

    let (ci, cj) = (i as i64, j as i64);
    #[inline(always)]
    fn hash(a: i64, b: i64) -> usize {
        let mut h = (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        h ^= h >> 29;
        (h >> 33) as usize
    }
    let g0 = grad2(hash(ci, cj));
    let g1 = grad2(hash(ci + oi as i64, cj + oj as i64));
    let g2 = grad2(hash(ci + 1, cj + 1));

    surflet(g0, dx0, dy0) + surflet(g1, dx1, dy1) + surflet(g2, dx2, dy2)
}


/// The crate's algorithm with the three `t > 0.0` tests replaced by a clamp.
///
/// A gradient lookup is harmless when a corner has no influence — the index is
/// in range whatever the distance — so the test buys nothing but a branch, and
/// which side it falls on is a property of where the sample landed in its
/// triangle, which is to say unpredictable.
#[inline(always)]
fn simplex_2d_branchless(point: [f64; 2], table: &[u8; 256]) -> f64 {
    let skew = (point[0] + point[1]) * SKEW;
    let (i, j) = ((point[0] + skew).floor(), (point[1] + skew).floor());
    let unskew = (i + j) * UNSKEW;
    let (dx0, dy0) = (point[0] - (i - unskew), point[1] - (j - unskew));

    let (oi, oj) = if dx0 > dy0 { (1.0, 0.0) } else { (0.0, 1.0) };
    let (dx1, dy1) = (dx0 - oi + UNSKEW, dy0 - oj + UNSKEW);
    let (dx2, dy2) = (dx0 - 1.0 + 2.0 * UNSKEW, dy0 - 1.0 + 2.0 * UNSKEW);

    let (ci, cj) = (i as isize, j as isize);
    let hash = |a: isize, b: isize| -> usize {
        let idx = table[(a & 0xff) as usize] as usize ^ (b & 0xff) as usize;
        table[idx] as usize
    };
    let g0 = grad2(hash(ci, cj));
    let g1 = grad2(hash(ci + oi as isize, cj + oj as isize));
    let g2 = grad2(hash(ci + 1, cj + 1));

    #[inline(always)]
    fn surflet(g: [f64; 2], dx: f64, dy: f64) -> f64 {
        let t = (1.0 - (dx * dx + dy * dy) * 2.0).max(0.0);
        let t2 = t * t;
        (2.0 * t2 + t2 * t2) * (dx * g[0] + dy * g[1])
    }
    surflet(g0, dx0, dy0) + surflet(g1, dx1, dy1) + surflet(g2, dx2, dy2)
}

/// The crate's exact call shape: out of line, as it is when the `noise` crate
/// is compiled separately and its `get` carries no `#[inline]`.
#[inline(never)]
fn crate_shape_out_of_line(point: [f64; 2], table: &[u8; 256]) -> f64 {
    simplex_2d_value(point, table)
}

/// The branchless version behind the same call, so the two are compared
/// without inlining confusing the difference.
#[inline(never)]
fn branchless_out_of_line(point: [f64; 2], table: &[u8; 256]) -> f64 {
    simplex_2d_branchless(point, table)
}


pub fn run() {
    let ph = phases();
    let simplex = Simplex::default();
    let arm = Arm {
        n_periods: 4.0,
        phase_temporal: 0.0,
        smoothing: 0.25,
        ticks: 0.0,
        scale: 0.6,
    };
    // A table of the same shape and distribution as the crate's, for the copies.
    let table: [u8; 256] = {
        let mut t = [0u8; 256];
        let mut state = 0x1234_5678_9abc_def0u64;
        for (n, slot) in t.iter_mut().enumerate() {
            *slot = n as u8;
            let _ = state;
        }
        // Fisher-Yates with the same xorshift the phases use.
        for n in (1..256).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            t.swap(n, (state % (n as u64 + 1)) as usize);
        }
        t
    };
    let _ = arm.scale;

    println!("{N} samples per pass, {REPEATS} passes\n");

    println!("--- through the real model ---");
    for (label, waveform) in [
        ("Waveform::Constant", Waveform::Constant),
        ("Waveform::Sine (untabled)", Waveform::Sine),
        ("Waveform::Triangle", Waveform::Triangle),
        ("Waveform::Noise", Waveform::Noise),
    ] {
        let p = prepared(waveform);
        time(&format!("PreparedAnimation::value, {label}"), || {
            let mut acc = 0.0;
            for (i, &phase) in ph.iter().enumerate() {
                acc += p.value(Phase::new(phase), i);
            }
            acc
        });
    }

    println!("\n--- the arm rebuilt, kernel swapped ---");
    time("arm arithmetic + stub kernel", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += stub(black_box(arm.point(phase, i)));
        }
        acc
    });
    time("arm arithmetic + noise::Simplex::get", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += simplex.get(black_box(arm.point(phase, i)));
        }
        acc
    });
    time("arm arithmetic + local copy, no derivative", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += simplex_2d_value(black_box(arm.point(phase, i)), &table);
        }
        acc
    });
    time("arm arithmetic + local copy, mix hash", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += simplex_2d_value_mixhash(black_box(arm.point(phase, i)));
        }
        acc
    });
    time("arm arithmetic + local copy, f32", || {
        let mut acc = 0.0f32;
        for (i, &phase) in ph.iter().enumerate() {
            let p = arm.point(phase, i);
            acc += simplex_2d_value_f32(black_box([p[0] as f32, p[1] as f32]), &table);
        }
        acc as f64
    });

    println!("\n--- how the second axis is walked ---");
    let sweep = |label: &str, y: &dyn Fn(usize) -> f64| {
        time(label, || {
            let mut acc = 0.0;
            for (i, &phase) in ph.iter().enumerate() {
                acc += simplex.get(black_box([phase * 4.0, y(i)]));
            }
            acc
        });
    };
    sweep("y = 0 (one row of the field)", &|_| 0.0);
    sweep("y = 0.75 * (i % 4) (four rows)", &|i| 0.75 * (i % 4) as f64);
    sweep("y = 0.75 * (i % 256) (one table period)", &|i| {
        0.75 * (i % 256) as f64
    });
    sweep("y = 0.75 * i (what the arm does)", &|i| 0.75 * i as f64);
    sweep("y = 1e6 + 0.75 * i (far from the origin)", &|i| {
        1e6 + 0.75 * i as f64
    });

    println!("\n--- the three surflet tests, kept and removed ---");
    time("inlined, branching (as the crate is written)", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += simplex_2d_value(black_box(arm.point(phase, i)), &table);
        }
        acc
    });
    time("inlined, branchless", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += simplex_2d_branchless(black_box(arm.point(phase, i)), &table);
        }
        acc
    });
    time("out of line, branching (the crate's call shape)", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += crate_shape_out_of_line(arm.point(phase, i), &table);
        }
        acc
    });
    time("out of line, branchless", || {
        let mut acc = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            acc += branchless_out_of_line(arm.point(phase, i), &table);
        }
        acc
    });

    println!("\n--- agreement (max |difference| against noise::Simplex) ---");
    let table_of_crate = {
        // The copies use their own table, so compare shapes, not values: what
        // matters is that a variant agrees with the branching version it was
        // derived from.
        let mut worst: f64 = 0.0;
        for (i, &phase) in ph.iter().enumerate() {
            let p = arm.point(phase, i);
            worst = worst.max((simplex_2d_value(p, &table) - simplex_2d_branchless(p, &table)).abs());
        }
        worst
    };
    println!("branchless vs branching: {table_of_crate:.3e}");

    // Against the crate's own `simplex_2d`, whatever `Simplex::get` is
    // currently compiled to call. Reports how far the two drift over the sweep
    // the arm actually walks, and how many samples differ at all.
    let reference = noise::permutationtable::PermutationTable::new(0);
    let (mut worst, mut differing) = (0.0f64, 0usize);
    for (i, &phase) in ph.iter().enumerate() {
        let p = arm.point(phase, i);
        let (want, _) = noise::core::simplex::simplex_2d(p.into(), &reference);
        let got = simplex.get(p);
        if got != want {
            differing += 1;
            worst = worst.max((got - want).abs());
        }
    }
    println!(
        "Simplex::get vs the crate's simplex_2d: {differing} of {N} samples differ, worst {worst:.3e}"
    );
}

// --- the same animation inside the real per-vertex pass -------------------

/// Runs the real `vertex_pass` over the heaviest mesh with one hue animation on
/// a second axis, which is the configuration `stats` reports as 43ns a vertex.
///
/// Isolated into its own command so a sampling profiler sees one workload for
/// the length of a recording.
pub fn in_situ(shape_dir: &std::path::Path, case: &str, seconds: f64) -> anyhow::Result<()> {
    use crate::draw::{AxisWave, VertexBuffers, VertexWork, vertex_pass};
    use crate::mesh::{self, Level, refine};
    use crate::params::{AnimTarget, ColorPhase, LayerParams, PhaseField, WaveParams};
    use crate::anim::{LiveWave, WaveformKind};

    let shapes = crate::shapes::load_dir(shape_dir)?;
    let heaviest = (0..shapes.len())
        .max_by_key(|&i| shapes[i].fill.len())
        .unwrap();
    let level = Level::for_scale(1.0, 1080.0, mesh::DEFAULT_TARGET_PX);
    let mesh = refine(&shapes[heaviest].fill, level.target_edge());
    let verts = mesh.verts.len();

    let layer = LayerParams {
        enabled: true,
        color_phase: ColorPhase::Angle,
        col_center: 0.45,
        col_width: 0.8,
        col_spread: 0.35,
        col_sat: 0.9,
        ..Default::default()
    };
    let waveform = match case {
        "noise" => WaveformKind::Noise,
        "sine" => WaveformKind::Sine,
        "triangle" => WaveformKind::Triangle,
        _ => WaveformKind::Sine,
    };
    let wave = LiveWave::new(&WaveParams {
        waveform,
        n_periods: 4,
        size: 0.6,
        ..Default::default()
    });
    let axes = if case == "baseline" {
        Vec::new()
    } else {
        vec![AxisWave {
            target: AnimTarget::Hue,
            phase: ColorPhase::Radius,
            wave: &wave,
        }]
    };

    let field = PhaseField::of(&layer);
    let mut buffers = VertexBuffers::default();
    let start = Instant::now();
    let mut passes = 0u64;
    while start.elapsed().as_secs_f64() < seconds {
        for _ in 0..16 {
            vertex_pass(
                &mut buffers,
                &mesh,
                VertexWork {
                    field,
                    base_spin: 0.0,
                    warps: &[],
                    hue_axes: &axes,
                    bright_axes: &[],
                },
            );
        }
        passes += 16;
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{case}: {verts} vertices, {passes} passes, {:.2}ns per vertex",
        elapsed * 1e9 / (passes as usize * verts) as f64
    );
    black_box(&buffers);
    Ok(())
}
