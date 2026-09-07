//! Drawing the same scene down both fill paths and diffing the pixels.
//!
//! The GL path is only worth having if it is indistinguishable from the CPU
//! one, and "looks the same to me" is not a claim a machine with no display can
//! make. This renders a scene twice into the same context — once through
//! piston, once through the shader — reads the framebuffer back after each, and
//! reports where they differ.
//!
//! Every scene is drawn a third time down the CPU path. Those two CPU frames
//! have to come back byte for byte identical, or the comparison is measuring
//! the renderer's own drift rather than the difference between the paths.

use crate::gpu::GpuRenderer;
use crate::params::{
    AnimTarget, ColorPhase, DemoParams, DrawMode, LayerParams, N_WAVES, TargetedWave, WaveParams,
};
use crate::anim::WaveformKind;
use crate::render::{Frame, Renderer};
use crate::shapes::load_dir;
use anyhow::{Result, anyhow};
use graphics::{Transformed, Viewport, clear};
use image::RgbaImage;
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;

pub struct Options {
    pub width: u32,
    pub height: u32,
    /// Multisampling. Off by default: an edge lands on a different set of
    /// samples for a sub-pixel change in a vertex, which would report every
    /// silhouette as a difference and say nothing about the shader.
    pub samples: u8,
    /// Where to write the frames of a scene that differs.
    pub out_dir: PathBuf,
    /// A channel difference at or below this is not worth a picture.
    pub tolerance: u8,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            samples: 0,
            out_dir: PathBuf::from("/tmp/svg_demo_compare"),
            tolerance: 2,
        }
    }
}

/// One thing the shader has to get right, and a scene that exercises it.
struct Scene {
    name: &'static str,
    params: DemoParams,
}

/// A layer with one animation slot filled in.
fn animated(base: LayerParams, target: AnimTarget, phase: ColorPhase, wave: WaveParams) -> LayerParams {
    let mut layer = base;
    layer.waves = vec![TargetedWave::default(); N_WAVES];
    layer.waves[0] = TargetedWave {
        enabled: true,
        target,
        phase,
        wave,
    };
    layer
}

/// A shape with a colour gradient across it, which is what puts a layer on the
/// ramp path rather than the flat one.
fn gradient(shape: usize) -> LayerParams {
    LayerParams {
        enabled: true,
        shape,
        col_width: 0.6,
        col_spread: 0.25,
        ..Default::default()
    }
}

fn wave(waveform: WaveformKind, size: f64) -> WaveParams {
    WaveParams {
        waveform,
        size,
        // Every clock stands still: the two paths have to be compared on the
        // same frame, not on adjacent ones.
        speed: 0.0,
        ..Default::default()
    }
}

fn one(layer: LayerParams) -> DemoParams {
    DemoParams {
        layers: vec![layer],
        gpu: false,
    }
}

/// The scenes, one per branch the shader can take.
fn scenes(shape: usize, stroke_shape: usize) -> Vec<Scene> {
    let mut out = Vec::new();
    let mut push = |name: &'static str, params: DemoParams| out.push(Scene { name, params });

    // The flat path: a single colour, no ramp, no displacement.
    push(
        "flat colour",
        one(LayerParams {
            enabled: true,
            shape,
            ..Default::default()
        }),
    );
    push(
        "mask",
        one(LayerParams {
            enabled: true,
            shape,
            mask: true,
            ..Default::default()
        }),
    );
    push(
        "flat with outline",
        one(LayerParams {
            enabled: true,
            shape: stroke_shape,
            draw_mode: DrawMode::Both,
            ..Default::default()
        }),
    );
    push(
        "level below full",
        one(LayerParams {
            enabled: true,
            shape,
            level: 0.4,
            ..Default::default()
        }),
    );

    // The ramp path, once per coordinate the phase can run along. Angle is the
    // one that wraps, so it is the one the branch attribute exists for.
    for phase in ColorPhase::ALL {
        push(
            match phase {
                ColorPhase::Angle => "ramp along angle",
                ColorPhase::Radius => "ramp along radius",
                ColorPhase::LinearX => "ramp along x",
                ColorPhase::LinearY => "ramp along y",
            },
            one(LayerParams {
                color_phase: phase,
                ..gradient(shape)
            }),
        );
    }

    // A high cycle count puts several ramp periods inside one triangle, which
    // is where a wrong unwrap period shows up.
    push(
        "ramp, many cycles",
        one(LayerParams {
            col_spread: 0.9,
            ..gradient(shape)
        }),
    );

    // The static spin knob, which is the only thing that sends an unanimated
    // layer through the polar round trip.
    push(
        "static spin",
        one(LayerParams {
            spin: 0.15,
            ..gradient(shape)
        }),
    );

    // Geometry animations, one per target.
    for (name, target) in [
        ("animated radial", AnimTarget::Radial),
        ("animated spin", AnimTarget::Spin),
        ("animated aspect", AnimTarget::AspectRatio),
    ] {
        push(
            name,
            one(animated(
                gradient(shape),
                target,
                ColorPhase::Angle,
                wave(WaveformKind::Sine, 0.4),
            )),
        );
    }

    // Colour on a second axis, which reaches the fragment through the vertices.
    push(
        "hue on a second axis",
        one(animated(
            gradient(shape),
            AnimTarget::Hue,
            ColorPhase::Radius,
            wave(WaveformKind::Sine, 0.5),
        )),
    );
    push(
        "brightness on a second axis",
        one(animated(
            gradient(shape),
            AnimTarget::Brightness,
            ColorPhase::Radius,
            wave(WaveformKind::Sine, 0.5),
        )),
    );

    // Every waveform the shader claims a closed form for, on a target where a
    // wrong one is visible as geometry rather than as a shade of a colour.
    for waveform in [
        WaveformKind::Sine,
        WaveformKind::Triangle,
        WaveformKind::Square,
        WaveformKind::Sawtooth,
        WaveformKind::Constant,
    ] {
        push(
            match waveform {
                WaveformKind::Sine => "waveform sine",
                WaveformKind::Triangle => "waveform triangle",
                WaveformKind::Square => "waveform square",
                WaveformKind::Sawtooth => "waveform sawtooth",
                _ => "waveform constant",
            },
            one(animated(
                gradient(shape),
                AnimTarget::Radial,
                ColorPhase::Angle,
                WaveParams {
                    n_periods: 5,
                    ..wave(waveform, 0.35)
                },
            )),
        );
    }

    // The switches on a waveform, each of which is a separate arm in the port.
    for (name, params) in [
        (
            "duty cycle",
            WaveParams {
                n_periods: 4,
                duty_cycle: 0.6,
                ..wave(WaveformKind::Sine, 0.35)
            },
        ),
        (
            "pulse",
            WaveParams {
                n_periods: 4,
                pulse: true,
                ..wave(WaveformKind::Triangle, 0.35)
            },
        ),
        (
            "standing",
            WaveParams {
                n_periods: 4,
                standing: true,
                ..wave(WaveformKind::Sine, 0.35)
            },
        ),
        (
            "invert",
            WaveParams {
                n_periods: 4,
                invert: true,
                ..wave(WaveformKind::Sawtooth, 0.35)
            },
        ),
        (
            "square with smoothing",
            WaveParams {
                n_periods: 4,
                smoothing: 0.7,
                ..wave(WaveformKind::Square, 0.35)
            },
        ),
        (
            "pulse square",
            WaveParams {
                n_periods: 4,
                pulse: true,
                smoothing: 0.5,
                ..wave(WaveformKind::Square, 0.35)
            },
        ),
    ] {
        push(
            name,
            one(animated(
                gradient(shape),
                AnimTarget::Radial,
                ColorPhase::Angle,
                params,
            )),
        );
    }

    // Noise has no closed form, so this layer must come back from the shader
    // path having been drawn by the CPU one — the control on the whole set.
    push(
        "noise falls back",
        one(animated(
            gradient(shape),
            AnimTarget::Radial,
            ColorPhase::Angle,
            wave(WaveformKind::Noise, 0.35),
        )),
    );

    // Two layers with a mask over them, which is where drawing order and
    // blending matter rather than any one layer's colour.
    push(
        "mask stacked over a gradient",
        DemoParams {
            layers: vec![
                gradient(shape),
                LayerParams {
                    enabled: true,
                    shape: stroke_shape,
                    mask: true,
                    scale_x: 0.5,
                    scale_y: 0.5,
                    ..Default::default()
                },
            ],
            gpu: false,
        },
    );

    // One frame with a layer on each path: the Noise layer falls back to
    // piston while the layer over it goes through the shader. This is the only
    // scene where a draw of ours lands in the middle of piston's batch, which
    // is the arrangement that corrupts it if the bracket is wrong. Both orders,
    // because a shader draw before piston's first draw of a frame and one after
    // it are different situations.
    let noisy = animated(
        gradient(shape),
        AnimTarget::Radial,
        ColorPhase::Angle,
        wave(WaveformKind::Noise, 0.35),
    );
    let shaded = LayerParams {
        scale_x: 0.5,
        scale_y: 0.5,
        ..animated(
            gradient(stroke_shape),
            AnimTarget::Radial,
            ColorPhase::Angle,
            wave(WaveformKind::Sine, 0.35),
        )
    };
    push(
        "cpu layer under a shader layer",
        DemoParams {
            layers: vec![noisy.clone(), shaded.clone()],
            gpu: false,
        },
    );
    push(
        "shader layer under a cpu layer",
        DemoParams {
            layers: vec![shaded, noisy],
            gpu: false,
        },
    );

    out
}

/// One frame, read back from the framebuffer.
fn render(
    renderer: &mut Renderer,
    gl: &mut GlGraphics,
    gpu: Option<&mut GpuRenderer>,
    params: &DemoParams,
    opts: &Options,
) -> RgbaImage {
    let (w, h) = (f64::from(opts.width), f64::from(opts.height));
    let viewport = Viewport {
        rect: [0, 0, opts.width as i32, opts.height as i32],
        draw_size: [opts.width, opts.height],
        window_size: [w, h],
    };
    gl.draw(viewport, |c, gl| {
        clear([0.0, 0.0, 0.0, 1.0], gl);
        let base = c.transform.trans(w / 2.0, h / 2.0);
        renderer.draw(
            gl,
            gpu,
            params,
            Frame {
                base,
                critical: w.min(h),
                // Held still, so the two paths are compared on one frame rather
                // than on consecutive ones.
                time: 0.0,
                delta: Duration::ZERO,
                audio: UnipolarFloat::ZERO,
            },
        );
    });
    read_back(opts.width, opts.height)
}

/// The back buffer, as an image with the origin at the top left.
fn read_back(width: u32, height: u32) -> RgbaImage {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    unsafe {
        gl::Finish();
        gl::ReadBuffer(gl::BACK);
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadPixels(
            0,
            0,
            width as i32,
            height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            buf.as_mut_ptr().cast(),
        );
    }
    let mut img = RgbaImage::from_raw(width, height, buf).unwrap_or_else(|| RgbaImage::new(width, height));
    // GL reads bottom up.
    image::imageops::flip_vertical_in_place(&mut img);
    img
}

/// How far apart two frames are.
struct Diff {
    /// The largest difference in any channel of any pixel.
    worst: u8,
    /// Pixels differing by more than the tolerance, in any channel.
    over: usize,
    total: usize,
}

impl Diff {
    fn of(a: &RgbaImage, b: &RgbaImage, tolerance: u8) -> Self {
        let mut worst = 0u8;
        let mut over = 0usize;
        for (pa, pb) in a.pixels().zip(b.pixels()) {
            let d = pa
                .0
                .iter()
                .zip(pb.0.iter())
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            worst = worst.max(d);
            if d > tolerance {
                over += 1;
            }
        }
        Self {
            worst,
            over,
            total: a.pixels().len(),
        }
    }

    fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.over as f64 / self.total as f64
    }
}

pub fn run(shape_dir: &Path, opts: Options) -> Result<()> {
    let shapes = load_dir(shape_dir)?;
    if shapes.len() < 2 {
        return Err(anyhow!("need at least two shapes, found {}", shapes.len()));
    }
    println!("comparing {} shapes loaded", shapes.len());

    let opengl = OpenGL::V3_2;
    let window: PistonWindow<Sdl2Window> =
        WindowSettings::new("svg_demo compare", [opts.width, opts.height])
            .graphics_api(opengl)
            .samples(opts.samples)
            .vsync(false)
            .exit_on_esc(true)
            .build()
            .map_err(|e| anyhow!("{e}"))?;
    // The window is only here to own a context. Nothing is ever presented, so
    // its event loop is never run and the frames are read out of the back
    // buffer instead.
    let _window = window;

    let mut gl = GlGraphics::new(opengl);
    let mut gpu = GpuRenderer::new().map_err(|e| anyhow!("gl fill path unavailable: {e}"))?;
    let mut renderer = Renderer::new(shapes);

    std::fs::create_dir_all(&opts.out_dir)?;

    // A shape with enough interior to show a gradient, and one with an outline
    // worth drawing.
    let scenes = scenes(0, 1);
    let mut differing = 0;
    let mut unstable = 0;

    println!(
        "\n{:<32} {:>7} {:>12} {:>8}",
        "scene", "worst", "pixels over", "of frame"
    );
    println!("{}", "─".repeat(62));

    for scene in &scenes {
        renderer.ensure_layers(scene.params.layers.len(), &scene.params.layers[0]);

        let cpu = render(&mut renderer, &mut gl, None, &scene.params, &opts);
        let shader = render(&mut renderer, &mut gl, Some(&mut gpu), &scene.params, &opts);
        let cpu_again = render(&mut renderer, &mut gl, None, &scene.params, &opts);

        // Without this the numbers below could be the renderer drifting between
        // frames rather than the two paths disagreeing.
        let drift = Diff::of(&cpu, &cpu_again, 0);
        if drift.worst != 0 {
            unstable += 1;
            println!(
                "{:<32} {:>7} {:>12} {:>8}  UNSTABLE: the cpu path differs from itself by {}",
                scene.name, "-", "-", "-", drift.worst
            );
            continue;
        }

        let diff = Diff::of(&cpu, &shader, opts.tolerance);
        println!(
            "{:<32} {:>7} {:>12} {:>7.3}%",
            scene.name,
            diff.worst,
            diff.over,
            diff.fraction() * 100.0
        );

        if diff.worst > opts.tolerance {
            differing += 1;
            let stem = scene.name.replace(' ', "_");
            cpu.save(opts.out_dir.join(format!("{stem}.cpu.png")))?;
            shader.save(opts.out_dir.join(format!("{stem}.glsl.png")))?;
        }
    }

    println!(
        "\n{} scenes, {} with any pixel over a difference of {}, {} unstable",
        scenes.len(),
        differing,
        opts.tolerance,
        unstable
    );
    if differing > 0 {
        println!("frames for those written to {}", opts.out_dir.display());
    }
    println!(
        "\nA silhouette is where the two paths are most likely to differ: the CPU\n\
         pass takes an angle through a fast approximation and the shader takes it\n\
         exactly, so a vertex can land a fraction of a pixel apart. Interior\n\
         disagreement is the kind that means the shader is wrong."
    );
    Ok(())
}
