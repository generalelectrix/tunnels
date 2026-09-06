//! Profiling the render path on real hardware.
//!
//! Runs the same `Renderer` the live window uses, with vsync off so frame times
//! reflect the work rather than the display, and reports where the time went.

use crate::params::{ColorPhase, DemoParams, DrawMode, LayerParams};
use crate::render::{Frame, Renderer, Timings};
use crate::shapes::load_dir;
use anyhow::{Result, anyhow};
use graphics::{Transformed, clear};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::path::Path;
use std::time::{Duration, Instant};
use tunnels_lib::number::UnipolarFloat;

/// Frames discarded before measuring, to let the driver settle and the mesh
/// cache fill.
const WARMUP_FRAMES: usize = 120;

pub struct Options {
    pub layers: usize,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    /// Shapes to draw. Empty means pick the heaviest in the library.
    pub shapes: Vec<usize>,
    /// Target on-screen triangle size. Triangle count goes as its inverse
    /// square, so this is the strongest lever on per-frame cost.
    pub target_px: f64,
    /// Multisampling. Free on a modern GPU, not necessarily on an old one.
    pub samples: u8,
    /// Frame rate the budget is measured against.
    ///
    /// Defaults to the client's own cap rather than a display refresh:
    /// `tunnelclient` sets `max_fps(120)` precisely because vsync is unreliable
    /// on some machines, so on those the loop is paced by that cap and a frame
    /// is 8.3ms, not 16.7ms.
    pub budget_hz: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            layers: 3,
            seconds: 6.0,
            width: 1920,
            height: 1080,
            shapes: Vec::new(),
            target_px: crate::mesh::DEFAULT_TARGET_PX,
            samples: 4,
            budget_hz: 120.0,
        }
    }
}

/// What one scenario measured.
struct Run {
    name: &'static str,
    frames: Vec<u128>,
    stages: Vec<Timings>,
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

fn us(v: u128) -> String {
    format!("{:.2}ms", v as f64 / 1000.0)
}

impl Run {
    fn report(&self, budget_us: f64) {
        if self.frames.is_empty() {
            println!("\n── {} ──\nno frames measured", self.name);
            return;
        }
        let mut frames = self.frames.clone();
        frames.sort_unstable();
        let p50 = percentile(&frames, 0.50);
        let p95 = percentile(&frames, 0.95);
        let p99 = percentile(&frames, 0.99);
        let max = *frames.last().unwrap_or(&0);
        let mean: f64 = frames.iter().map(|v| *v as f64).sum::<f64>() / frames.len() as f64;

        let stage = |f: fn(&Timings) -> u128| {
            let mut v: Vec<u128> = self.stages.iter().map(f).collect();
            v.sort_unstable();
            (percentile(&v, 0.50), *v.last().unwrap_or(&0))
        };
        let (ramp_p50, ramp_max) = stage(|t| t.ramp_us);
        let (mesh_p50, mesh_max) = stage(|t| t.mesh_us);
        let (uv_p50, uv_max) = stage(|t| t.uv_us);
        let (submit_p50, submit_max) = stage(|t| t.submit_us);
        let (cpu_p50, cpu_max) = stage(|t| t.cpu_us());
        let tris = self
            .stages
            .iter()
            .map(|t| t.triangles)
            .max()
            .unwrap_or(0);

        println!("\n── {} ──────────────────────────────", self.name);
        println!("{} frames, {} triangles/frame", frames.len(), tris);
        println!(
            "frame   p50 {}  p95 {}  p99 {}  max {}   ({:.0} fps mean)",
            us(p50),
            us(p95),
            us(p99),
            us(max),
            1e6 / mean
        );
        println!("  cpu    p50 {}  max {}", us(cpu_p50), us(cpu_max));
        println!("    phase eval   p50 {}  max {}", us(uv_p50), us(uv_max));
        println!("    submit       p50 {}  max {}", us(submit_p50), us(submit_max));
        println!("    ramp build   p50 {}  max {}", us(ramp_p50), us(ramp_max));
        println!("    mesh build   p50 {}  max {}", us(mesh_p50), us(mesh_max));
        // Whatever the frame took beyond our own CPU work is the driver, the
        // GPU, and the buffer swap.
        println!(
            "  gpu + swap (frame - cpu)   p50 {}",
            us(p50.saturating_sub(cpu_p50))
        );
        let headroom = budget_us / p99.max(1) as f64;
        println!(
            "p99 uses {:.0}% of a {:.1}ms frame — {:.1}x headroom",
            p99 as f64 / budget_us * 100.0,
            budget_us / 1000.0,
            headroom
        );
    }
}

/// A layer set built to be pessimistic: big shapes, gradients on, spinning.
fn scene(shapes: &[usize]) -> DemoParams {
    let layers = shapes
        .iter()
        .enumerate()
        .map(|(i, &shape)| LayerParams {
            enabled: true,
            shape,
            // Near-full-screen, which selects the finest mesh level in play.
            scale_x: 1.0,
            scale_y: 1.0,
            spin_speed: 0.05 + 0.01 * i as f64,
            draw_mode: DrawMode::Fill,
            color_phase: ColorPhase::Angle,
            col_center: 0.45,
            col_width: 0.8,
            // Five cycles, so the color moves fast everywhere.
            col_spread: 0.35,
            col_sat: 0.9,
            ..Default::default()
        })
        .collect();
    DemoParams { layers }
}

pub fn run(shape_dir: &Path, opts: Options) -> Result<()> {
    let shapes = load_dir(shape_dir)?;
    println!("profiling {} shapes from the library", shapes.len());

    // Default to the heaviest shapes by source triangle count — the meshes
    // built from them are the largest, so this is the pessimistic pick.
    let picks: Vec<usize> = if opts.shapes.is_empty() {
        let mut idx: Vec<usize> = (0..shapes.len()).collect();
        idx.sort_by_key(|&i| std::cmp::Reverse(shapes[i].fill.len()));
        idx.into_iter().take(opts.layers.max(1)).collect()
    } else {
        opts.shapes.iter().copied().filter(|i| *i < shapes.len()).collect()
    };
    if picks.is_empty() {
        return Err(anyhow!("no shapes to profile"));
    }
    println!("layers: {}", picks.iter().map(|&i| shapes[i].name.as_str()).collect::<Vec<_>>().join(", "));

    let opengl = OpenGL::V3_2;
    let mut window: PistonWindow<Sdl2Window> =
        WindowSettings::new("svg_demo: profile", [opts.width, opts.height])
            .graphics_api(opengl)
            .exit_on_esc(true)
            // Vsync off: with it on this measures the display, not the work.
            .vsync(false)
            .samples(opts.samples)
            .build()
            .map_err(|e| anyhow!("{e}"))?;
    window.set_max_fps(100_000);
    let mut gl = GlGraphics::new(opengl);
    let mut renderer = Renderer::new(shapes);
    renderer.target_px = opts.target_px;

    let mut runs = Vec::new();
    for (name, animate_color) in [("color animating", true), ("color held still", false)] {
        let mut params = scene(&picks);
        renderer.ensure_layers(params.layers.len(), &params.layers[0]);

        let mut frames = Vec::new();
        let mut stages = Vec::new();
        let start = Instant::now();
        let mut last = Instant::now();
        let mut seen = 0usize;
        let deadline = Duration::from_secs_f64(opts.seconds / 2.0);

        for e in window.by_ref() {
            let Some(args) = e.render_args() else { continue };
            let time = start.elapsed().as_secs_f64();
            if animate_color {
                // Sweep hue so the ramp is rebuilt on every single frame.
                for (i, l) in params.layers.iter_mut().enumerate() {
                    l.col_center = (time * 0.37 + i as f64 * 0.1).rem_euclid(1.0);
                }
            }

            let (w, h) = (args.window_size[0], args.window_size[1]);
            let critical = w.min(h);
            let elapsed_hint = last.elapsed();
            let mut t = Timings::default();
            gl.draw(args.viewport(), |c, gl| {
                clear([0.0, 0.0, 0.0, 1.0], gl);
                let base = c.transform.trans(w / 2.0, h / 2.0);
                t = renderer.draw(
                    gl,
                    &params,
                    Frame {
                        base,
                        critical,
                        time,
                        delta: elapsed_hint,
                        audio: UnipolarFloat::ZERO,
                    },
                );
            });

            let elapsed = last.elapsed();
            last = Instant::now();
            seen += 1;
            if seen > WARMUP_FRAMES {
                frames.push(elapsed.as_micros());
                stages.push(t);
                if start.elapsed() > deadline {
                    break;
                }
            }
        }
        runs.push(Run {
            name,
            frames,
            stages,
        });
    }

    println!(
        "\n{}x{}, {} layers, {}x msaa, {:.0}px triangles, vsync off, {} warmup frames discarded",
        opts.width,
        opts.height,
        picks.len(),
        opts.samples,
        opts.target_px,
        WARMUP_FRAMES
    );
    println!(
        "mesh library after run: {} meshes, {} triangles",
        renderer.mesh_count(),
        renderer.mesh_triangles()
    );
    for run in &runs {
        run.report(1e6 / opts.budget_hz);
    }
    println!(
        "\nBudget is one frame at {:.0}Hz. Where vsync works, that is the \n\
         projector's refresh and the tail is absorbed by waiting for it. Where \n\
         it does not, the loop is paced by tunnelclient's own max_fps cap and \n\
         nothing absorbs the tail, so p99 matters more than p50.",
        opts.budget_hz
    );
    Ok(())
}
