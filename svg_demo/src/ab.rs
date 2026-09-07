//! The two fill paths, head to head, in one run.
//!
//! `profile` measures one path. Answering "is the shader worth it" with it
//! means running it twice and diffing two tables by eye, on two different
//! minutes of the machine's life. This runs both against the same scene and
//! prints the comparison.
//!
//! # Why the blocks alternate
//!
//! A laptop throttles and a desktop has other work on it, so a run that does
//! all of one path and then all of the other charges whatever drifted during it
//! to whichever path went second. The two paths take short turns instead, so a
//! drift is spread across both rather than landing on one.
//!
//! # Why the renderer string is the first thing printed
//!
//! A machine with no GPU driver still gives you a working OpenGL context — a
//! software rasteriser — and it produces a full set of plausible numbers that
//! mean nothing about the hardware this is asking about. The only defence is to
//! say which renderer answered.

use crate::gpu::GpuRenderer;
use crate::params::{ColorPhase, DemoParams, DrawMode, LayerParams};
use crate::render::{Frame, Renderer, Timings};
use crate::shapes::{ShapeMesh, load_dir};
use anyhow::{Result, anyhow};
use graphics::{Transformed, clear};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::path::Path;
use std::time::{Duration, Instant};
use tunnels_lib::number::UnipolarFloat;

/// Frames discarded when a layer count changes, letting the mesh cache fill and
/// the driver settle.
const WARMUP_FRAMES: usize = 120;

/// Frames discarded after each switch between the paths.
///
/// The first frame on a path pays for what the other one invalidated — a
/// program rebind, or the upload of a vertex buffer this scene has not used
/// yet. Charging that to the path's steady state would be measuring the switch.
const SWITCH_FRAMES: usize = 10;

/// Frames a block records before its clock is allowed to end it.
///
/// A block short enough to hold two frames reports a p99 that is just its
/// slowest frame. The floor costs a slow configuration some wall time and buys
/// a percentile that means something.
const MIN_BLOCK_FRAMES: usize = 12;

/// Frames a cell wants before its percentiles are worth quoting.
const THIN_SAMPLE: usize = 40;

/// Hue moved per frame in the animating half, in turns.
///
/// Driven by the frame rather than by the clock, because `RampKey` quantises to
/// half a texel and a fast machine sweeping by wall time steps inside one
/// bucket and rebuilds nothing — which would quietly turn the animating case
/// into a second copy of the still one on exactly the hardware where the
/// question matters. This is four quantisation steps, so the rebuild happens
/// however fast the frames arrive.
const HUE_PER_FRAME: f64 = 1.0 / 512.0;

pub struct Options {
    /// Layer counts to sweep. One scene answers one question; the useful axis
    /// is how each path's cost grows.
    pub layers: Vec<usize>,
    /// Measured seconds per layer count per colour mode, split evenly between
    /// the two paths.
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    /// Shapes to draw from. Empty means the heaviest in the library.
    pub shapes: Vec<usize>,
    pub target_px: f64,
    pub samples: u8,
    /// Turns each path takes per colour mode. More turns spread a drift more
    /// finely and cost more switches.
    pub blocks: usize,
    /// Frame rate the budget is measured against. `tunnelclient` caps itself at
    /// 120 because vsync is unreliable on some machines, and on those the cap
    /// is what paces the loop.
    pub budget_hz: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            layers: vec![1, 3, 6],
            seconds: 4.0,
            width: 1920,
            height: 1080,
            shapes: Vec::new(),
            target_px: crate::mesh::DEFAULT_TARGET_PX,
            samples: 4,
            blocks: 4,
            budget_hz: 120.0,
        }
    }
}

/// Which fill path drew a frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    Cpu,
    Glsl,
}

/// The frames one path drew in one configuration.
#[derive(Default)]
struct Cell {
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

/// One path's numbers, reduced from its frames.
#[derive(Default, Clone, Copy)]
struct Stats {
    frames: usize,
    triangles: usize,
    p50: u128,
    p95: u128,
    p99: u128,
    max: u128,
    cpu: u128,
    phase: u128,
    submit: u128,
    ramp: u128,
    mesh: u128,
}

impl Stats {
    /// Whatever a frame took beyond our own CPU work: the driver, the GPU and
    /// the buffer swap.
    fn gpu_and_swap(&self) -> u128 {
        self.p50.saturating_sub(self.cpu)
    }
}

impl Cell {
    fn stats(&self) -> Stats {
        if self.frames.is_empty() {
            return Stats::default();
        }
        let mut frames = self.frames.clone();
        frames.sort_unstable();
        let stage = |f: fn(&Timings) -> u128| {
            let mut v: Vec<u128> = self.stages.iter().map(f).collect();
            v.sort_unstable();
            percentile(&v, 0.50)
        };
        Stats {
            frames: frames.len(),
            triangles: self.stages.iter().map(|t| t.triangles).max().unwrap_or(0),
            p50: percentile(&frames, 0.50),
            p95: percentile(&frames, 0.95),
            p99: percentile(&frames, 0.99),
            max: *frames.last().unwrap_or(&0),
            cpu: stage(Timings::cpu_us),
            phase: stage(|t| t.uv_us),
            submit: stage(|t| t.submit_us),
            ramp: stage(|t| t.ramp_us),
            mesh: stage(|t| t.mesh_us),
        }
    }
}

fn ms(v: u128) -> String {
    format!("{:.2}ms", v as f64 / 1000.0)
}

/// The change from the CPU path to the shader, as a percentage.
///
/// Below a hundredth of a millisecond a percentage is noise amplified into a
/// headline, so the comparison is declined rather than invented.
fn delta(base: u128, new: u128) -> String {
    if base < 10 && new < 10 {
        return "—".to_string();
    }
    if base == 0 {
        return format!("+{}", ms(new));
    }
    let pct = (new as f64 - base as f64) / base as f64 * 100.0;
    format!("{pct:+.0}%")
}

/// One line of the side-by-side table.
fn row(name: &str, cpu: u128, glsl: u128) {
    println!(
        "  {:<24} {:>9} {:>10} {:>9}",
        name,
        ms(cpu),
        ms(glsl),
        delta(cpu, glsl)
    );
}

/// A scene built to be pessimistic: big shapes, gradients on, rotating.
fn scene(shapes: &[usize]) -> DemoParams {
    let layers = shapes
        .iter()
        .enumerate()
        .map(|(i, &shape)| LayerParams {
            enabled: true,
            shape,
            scale_x: 1.0,
            scale_y: 1.0,
            rot_speed: 0.05 + 0.01 * i as f64,
            draw_mode: DrawMode::Fill,
            color_phase: ColorPhase::Angle,
            col_center: 0.45,
            col_width: 0.8,
            col_spread: 0.35,
            col_sat: 0.9,
            ..Default::default()
        })
        .collect();
    DemoParams { layers, gpu: false }
}

/// What the driver calls itself.
fn gl_string(name: gl::types::GLenum) -> String {
    unsafe {
        let p = gl::GetString(name);
        if p.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr(p.cast())
            .to_string_lossy()
            .into_owned()
    }
}

/// Whether the renderer is rasterising on the CPU.
///
/// Every one of these produces a complete and entirely plausible set of frame
/// times that say nothing about any GPU.
fn is_software(renderer: &str, vendor: &str) -> bool {
    let haystack = format!("{renderer} {vendor}").to_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swrast",
        "software rasterizer",
        "software renderer",
        "swiftshader",
        "mesa offscreen",
        "basic render",
        "lavapipe",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

/// The window, the renderer and the caches, kept together so a block of frames
/// is one call rather than eleven arguments.
///
/// **`window` is declared last on purpose.** Struct fields drop in declaration
/// order, and the window owns the GL context. `GlGraphics` deletes its programs
/// and vertex arrays when it drops, so a context torn down before it takes
/// those calls with no context current and segfaults on the way out. The rest
/// of the crate gets this for free by keeping the two in locals, which drop in
/// reverse.
struct Bench {
    gpu: GpuRenderer,
    gl: GlGraphics,
    renderer: Renderer,
    start: Instant,
    last: Instant,
    /// Frames drawn since the run began, which drives the hue sweep.
    drawn: u64,
    window: PistonWindow<Sdl2Window>,
}

/// One turn at the machine.
struct Block<'a> {
    params: &'a mut DemoParams,
    path: Fill,
    /// Sweep hue every frame, so the ramp is rebuilt every frame.
    animate: bool,
    duration: Duration,
    /// Frames to throw away before recording.
    discard: usize,
}

impl Bench {
    /// Draw for a while down one path, recording what it cost.
    ///
    /// `out` is `None` for a warmup block, which draws exactly the same frames
    /// and keeps none of them.
    fn block(&mut self, spec: Block, mut out: Option<&mut Cell>) {
        let Block {
            params,
            path,
            animate,
            duration,
            discard,
        } = spec;
        let mut began = Instant::now();
        let mut seen = 0usize;
        let mut recorded = 0usize;

        for e in self.window.by_ref() {
            let Some(args) = e.render_args() else { continue };
            let time = self.start.elapsed().as_secs_f64();
            self.drawn += 1;
            if animate {
                let swept = self.drawn as f64 * HUE_PER_FRAME;
                for (i, l) in params.layers.iter_mut().enumerate() {
                    l.col_center = (swept + i as f64 * 0.1).rem_euclid(1.0);
                }
            }

            let (w, h) = (args.window_size[0], args.window_size[1]);
            let elapsed_hint = self.last.elapsed();
            let mut t = Timings::default();
            // The same `Renderer` the window runs, reached the same way, so a
            // number here is a number the show would see.
            let pass = match path {
                Fill::Glsl => Some(&mut self.gpu),
                Fill::Cpu => None,
            };
            let renderer = &mut self.renderer;
            self.gl.draw(args.viewport(), |c, gl| {
                clear([0.0, 0.0, 0.0, 1.0], gl);
                let base = c.transform.trans(w / 2.0, h / 2.0);
                t = renderer.draw(
                    gl,
                    pass,
                    params,
                    Frame {
                        base,
                        critical: w.min(h),
                        time,
                        delta: elapsed_hint,
                        audio: UnipolarFloat::ZERO,
                    },
                );
            });

            let elapsed = self.last.elapsed();
            self.last = Instant::now();
            seen += 1;
            if seen <= discard {
                // The clock starts when recording does. Otherwise a slow warmup
                // — and the first frame back on a path is the slowest one it
                // ever draws — eats the window it was supposed to precede.
                began = Instant::now();
                continue;
            }
            recorded += 1;
            if let Some(cell) = out.as_deref_mut() {
                cell.frames.push(elapsed.as_micros());
                cell.stages.push(t);
            }
            if recorded >= MIN_BLOCK_FRAMES && began.elapsed() > duration {
                return;
            }
        }
    }
}

/// The heaviest shapes in the library, which build the largest meshes.
fn pick(shapes: &[ShapeMesh], chosen: &[usize], n: usize) -> Vec<usize> {
    if !chosen.is_empty() {
        let pool: Vec<usize> = chosen.iter().copied().filter(|i| *i < shapes.len()).collect();
        // Repeat the pool rather than truncating it, so a sweep past its length
        // still asks for the layer count it was told to.
        return (0..n).filter_map(|i| pool.get(i % pool.len()).copied()).collect();
    }
    let mut idx: Vec<usize> = (0..shapes.len()).collect();
    idx.sort_by_key(|&i| std::cmp::Reverse(shapes[i].fill.len()));
    (0..n).filter_map(|i| idx.get(i % idx.len().max(1)).copied()).collect()
}

/// One colour mode at one layer count, both paths.
fn report_pair(name: &str, cpu: Stats, glsl: Stats, budget_us: f64) {
    println!("\n  {name:<24} {:>9} {:>10} {:>9}", "cpu", "glsl", "delta");
    row("frame p50", cpu.p50, glsl.p50);
    row("frame p95", cpu.p95, glsl.p95);
    row("frame p99", cpu.p99, glsl.p99);
    row("frame max", cpu.max, glsl.max);
    row("cpu total", cpu.cpu, glsl.cpu);
    row("  phase eval", cpu.phase, glsl.phase);
    row("  submit", cpu.submit, glsl.submit);
    row("  ramp build", cpu.ramp, glsl.ramp);
    row("  mesh build", cpu.mesh, glsl.mesh);
    row("gpu + swap", cpu.gpu_and_swap(), glsl.gpu_and_swap());
    println!(
        "  {:<24} {:>8.0}% {:>9.0}% {:>9}",
        "p99 of budget",
        cpu.p99 as f64 / budget_us * 100.0,
        glsl.p99 as f64 / budget_us * 100.0,
        delta(cpu.p99, glsl.p99)
    );
    println!(
        "  {:<24} {:>9} {:>10}",
        "frames measured", cpu.frames, glsl.frames
    );
    if cpu.frames < THIN_SAMPLE || glsl.frames < THIN_SAMPLE {
        println!(
            "  ^ under {THIN_SAMPLE} frames: read p50, not p99 — the tail here is \
             one or two frames. Raise --seconds."
        );
    }
}

pub fn run(shape_dir: &Path, opts: Options) -> Result<()> {
    let shapes = load_dir(shape_dir)?;
    if shapes.is_empty() {
        return Err(anyhow!("no shapes found in {}", shape_dir.display()));
    }
    let mut sweep = opts.layers.clone();
    sweep.retain(|n| *n > 0);
    sweep.sort_unstable();
    sweep.dedup();
    if sweep.is_empty() {
        return Err(anyhow!("no layer counts to sweep"));
    }

    let opengl = OpenGL::V3_2;
    let mut window: PistonWindow<Sdl2Window> =
        WindowSettings::new("svg_demo: cpu vs glsl", [opts.width, opts.height])
            .graphics_api(opengl)
            .exit_on_esc(true)
            // With vsync on this measures the display rather than the work.
            .vsync(false)
            .samples(opts.samples)
            .build()
            .map_err(|e| anyhow!("{e}"))?;
    window.set_max_fps(100_000);
    let gl = GlGraphics::new(opengl);

    let renderer_name = gl_string(gl::RENDERER);
    let vendor = gl_string(gl::VENDOR);
    let version = gl_string(gl::VERSION);
    println!("svg_demo: cpu vs glsl fill\n");
    println!("  renderer  {renderer_name}");
    println!("  vendor    {vendor}");
    println!("  gl        {version}");
    let software = is_software(&renderer_name, &vendor);
    if software {
        println!(
            "\n\
             ####################################################################\n\
             ##  THIS IS A SOFTWARE RASTERISER. The numbers below are not about ##\n\
             ##  any GPU. Every stage, including the vertex shader, is running  ##\n\
             ##  on the CPU, so the shader path is being charged for work that  ##\n\
             ##  is free on real hardware. Do not compare these to a machine    ##\n\
             ##  with a driver, and do not use them to decide anything.         ##\n\
             ####################################################################"
        );
    }

    let gpu = GpuRenderer::new().map_err(|e| {
        anyhow!("gl fill path unavailable, so there is nothing to compare against: {e}")
    })?;
    // Chosen before the library is handed to the renderer, which takes it.
    let picks: Vec<(usize, Vec<usize>)> = sweep
        .iter()
        .map(|&n| (n, pick(&shapes, &opts.shapes, n)))
        .collect();
    let mut renderer = Renderer::new(shapes);
    renderer.target_px = opts.target_px;
    let mut bench = Bench {
        gpu,
        gl,
        renderer,
        start: Instant::now(),
        last: Instant::now(),
        drawn: 0,
        window,
    };

    let budget_us = 1e6 / opts.budget_hz;
    let block_dur = Duration::from_secs_f64(opts.seconds / (2.0 * opts.blocks.max(1) as f64));
    println!(
        "\n{}x{}, {}x msaa, {:.0}px triangles, vsync off, budget {:.1}ms ({:.0}Hz)",
        opts.width,
        opts.height,
        opts.samples,
        opts.target_px,
        budget_us / 1000.0,
        opts.budget_hz
    );
    println!(
        "{} blocks of {:.0}ms per path, alternating; {WARMUP_FRAMES} warmup frames \
         per layer count and {SWITCH_FRAMES} after each switch",
        opts.blocks,
        block_dur.as_secs_f64() * 1000.0
    );

    // Kept for the closing summary, which is where a slope shows up.
    let mut summary: Vec<(usize, Stats, Stats)> = Vec::new();

    for (n, layer_shapes) in &picks {
        let n = *n;
        let mut params = scene(layer_shapes);
        bench
            .renderer
            .ensure_layers(params.layers.len(), &params.layers[0]);

        // Build every mesh and upload every buffer before anything is recorded,
        // so neither path is charged for the other's first use of a shape.
        for path in [Fill::Cpu, Fill::Glsl] {
            bench.block(
                Block {
                    params: &mut params,
                    path,
                    animate: false,
                    duration: Duration::ZERO,
                    discard: WARMUP_FRAMES,
                },
                None,
            );
        }

        println!("\n═══ {n} layers ═══════════════════════════════════════════════");

        let mut held = (Cell::default(), Cell::default());
        let mut moving = (Cell::default(), Cell::default());
        for _ in 0..opts.blocks.max(1) {
            for (animate, cells) in [(false, &mut held), (true, &mut moving)] {
                for path in [Fill::Cpu, Fill::Glsl] {
                    let cell = match path {
                        Fill::Cpu => &mut cells.0,
                        Fill::Glsl => &mut cells.1,
                    };
                    bench.block(
                        Block {
                            params: &mut params,
                            path,
                            animate,
                            duration: block_dur,
                            discard: SWITCH_FRAMES,
                        },
                        Some(cell),
                    );
                }
            }
        }

        let (held_cpu, held_glsl) = (held.0.stats(), held.1.stats());
        let (moving_cpu, moving_glsl) = (moving.0.stats(), moving.1.stats());
        println!(
            "  {} triangles/frame",
            held_cpu.triangles.max(held_glsl.triangles)
        );
        report_pair("colour held still", held_cpu, held_glsl, budget_us);
        report_pair("colour animating", moving_cpu, moving_glsl, budget_us);
        summary.push((n, held_cpu, held_glsl));
    }

    println!("\n═══ frame p50 against layer count, colour held still ═════════");
    println!("  {:>6} {:>9} {:>10} {:>9}", "layers", "cpu", "glsl", "delta");
    for (n, cpu, glsl) in &summary {
        println!(
            "  {n:>6} {:>9} {:>10} {:>9}",
            ms(cpu.p50),
            ms(glsl.p50),
            delta(cpu.p50, glsl.p50)
        );
    }
    println!(
        "\nThe slope is the answer, not any one row: what matters is where each \n\
         path stops fitting the budget, and whether they get there at the same \n\
         rate. Budget is one frame at {:.0}Hz — where vsync works that is the \n\
         projector's refresh and the tail is absorbed by waiting for it, and \n\
         where it does not the loop is paced by tunnelclient's own max_fps cap \n\
         and nothing absorbs the tail, so p99 matters more than p50.",
        opts.budget_hz
    );
    if software {
        println!(
            "\nAll of the above came from a software rasteriser and answers no \n\
             question about a GPU. Run it again on the render client."
        );
    }
    Ok(())
}
