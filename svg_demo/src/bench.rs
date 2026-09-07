//! A synthetic workload for sampling profilers.
//!
//! Runs exactly the per-frame CPU pipeline the render window runs — animations
//! ticked, ramp rebuilt, colour offsets and warps evaluated per vertex,
//! vertices projected and gathered into the backend's layout — with the
//! rasteriser replaced by a sink. What is left is the work a profiler should be
//! attributing, without a GPU or a display in the way.

use crate::anim::{LiveWave, WaveformKind};
use crate::draw::{
    AxisWave, Shaded, VertexBuffers, VertexWork, draw_textured, vertex_pass,
};
use crate::mesh::{self, Level, RefinedMesh, refine};
use crate::params::{
    AnimTarget, ColorPhase, LayerParams, PhaseField, TargetedWave, WaveParams,
};
use crate::ramp;
use crate::shapes::{ShapeMesh, load_dir};
use crate::software::RenderBuffer;
use anyhow::Result;
use graphics::draw_state::DrawState;
use graphics::{Graphics, ImageSize};
use graphics::math::identity;
use graphics::Transformed;
use std::path::Path;
use std::time::{Duration, Instant};
use tunnels_lib::number::UnipolarFloat;

/// A `Graphics` that does everything up to the pixels and then drops them.
///
/// The gather that fills its buffers is real work the client also does; what it
/// skips is the rasterisation, which on a GPU is not the CPU's problem at all.
struct Sink {
    vertices: usize,
}

impl ImageSize for Sink {
    fn get_size(&self) -> (u32, u32) {
        (1920, 1080)
    }
}

impl Graphics for Sink {
    type Texture = RenderBuffer;

    fn clear_color(&mut self, _: [f32; 4]) {}
    fn clear_stencil(&mut self, _: u8) {}

    fn tri_list<F>(&mut self, _: &DrawState, _: &[f32; 4], mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]])),
    {
        f(&mut |v| self.vertices += v.len());
    }

    fn tri_list_c<F>(&mut self, _: &DrawState, mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 4]])),
    {
        f(&mut |v, _| self.vertices += v.len());
    }

    fn tri_list_uv<F>(&mut self, _: &DrawState, _: &[f32; 4], _: &Self::Texture, mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]])),
    {
        f(&mut |v, _| self.vertices += v.len());
    }

    fn tri_list_uv_c<F>(&mut self, _: &DrawState, _: &Self::Texture, mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]], &[[f32; 4]])),
    {
        f(&mut |v, _, _| self.vertices += v.len());
    }
}

/// One layer's worth of everything the per-frame pipeline needs.
struct Layer {
    params: LayerParams,
    mesh: RefinedMesh,
    waves: Vec<LiveWave>,
    ramp: image::RgbaImage,
    verts: VertexBuffers,
}

/// A layer carrying the animation load worth profiling: a colour animation on
/// the ramp axis, another on a second axis, and a geometry warp.
fn layer_params(shape: usize) -> LayerParams {
    let mut params = LayerParams {
        enabled: true,
        shape,
        scale_x: 1.0,
        scale_y: 1.0,
        color_phase: ColorPhase::Angle,
        col_center: 0.45,
        col_width: 0.7,
        col_spread: 0.3,
        col_sat: 0.9,
        ..Default::default()
    };
    let wave = |waveform, n_periods| WaveParams {
        waveform,
        n_periods,
        size: 0.6,
        speed: 0.2,
        ..Default::default()
    };
    params.waves[0] = TargetedWave {
        enabled: true,
        target: AnimTarget::Hue,
        phase: ColorPhase::Angle,
        wave: wave(WaveformKind::Sawtooth, 3),
    };
    params.waves[1] = TargetedWave {
        enabled: true,
        target: AnimTarget::Brightness,
        phase: ColorPhase::Radius,
        wave: wave(WaveformKind::Sine, 4),
    };
    params.waves[2] = TargetedWave {
        enabled: true,
        target: AnimTarget::Spin,
        phase: ColorPhase::Radius,
        wave: wave(WaveformKind::Triangle, 2),
    };
    params
}

pub fn run(shape_dir: &Path, seconds: f64, n_layers: usize) -> Result<()> {
    let shapes = load_dir(shape_dir)?;
    let mut heavy: Vec<usize> = (0..shapes.len()).collect();
    heavy.sort_by_key(|&i| std::cmp::Reverse(shapes[i].fill.len()));
    let picks: Vec<usize> = heavy.into_iter().take(n_layers.max(1)).collect();

    // The density a full-screen shape on a 1080-line projector asks for.
    let level = Level::for_scale(1.0, 1080.0, mesh::DEFAULT_TARGET_PX);
    let mut layers: Vec<Layer> = picks
        .iter()
        .map(|&shape| {
            let params = layer_params(shape);
            Layer {
                mesh: refine(&shapes[shape].fill, level.target_edge()),
                waves: params.waves.iter().map(|w| LiveWave::new(&w.wave)).collect(),
                ramp: ramp::build(&params),
                params,
                verts: VertexBuffers::default(),
            }
        })
        .collect();

    let verts: usize = layers.iter().map(|l| l.mesh.verts.len()).sum();
    let tris: usize = layers.iter().map(|l| l.mesh.triangle_count()).sum();
    println!(
        "{} layers, {tris} triangles, {verts} vertices, level {level:?}",
        layers.len()
    );
    println!("shapes: {}", picks.iter().map(|&i| shapes[i].name.as_str()).collect::<Vec<_>>().join(", "));

    let texture = RenderBuffer::from_image(ramp::build(&layers[0].params));
    let mut sink = Sink { vertices: 0 };
    let base = identity().trans(960.0, 540.0);
    let audio = UnipolarFloat::ZERO;
    let delta = Duration::from_micros(8333);

    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed().as_secs_f64() < seconds {
        for layer in &mut layers {
            frame(layer, base, &texture, audio, delta, &mut sink);
        }
        frames += 1;
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{frames} frames in {elapsed:.2}s — {:.3}ms per frame for {} layers",
        elapsed * 1000.0 / frames as f64,
        layers.len()
    );
    std::hint::black_box(sink.vertices);
    Ok(())
}

/// One layer, one frame: everything the render loop does before the GPU.
fn frame(
    layer: &mut Layer,
    base: graphics::math::Matrix2d,
    texture: &RenderBuffer,
    audio: UnipolarFloat,
    delta: Duration,
    sink: &mut Sink,
) {
    for (live, slot) in layer.waves.iter_mut().zip(layer.params.waves.iter()) {
        live.update(&slot.wave, delta, audio);
    }

    let on_axis: Vec<_> = layer
        .params
        .waves
        .iter()
        .enumerate()
        .filter(|(_, w)| w.target.is_color() && w.phase == layer.params.color_phase)
        .map(|(i, w)| (w.target, &layer.waves[i]))
        .collect();
    ramp::build_into(&mut layer.ramp, &layer.params, &on_axis, audio);

    let pick = |keep: &dyn Fn(&TargetedWave) -> bool| -> Vec<AxisWave> {
        layer
            .params
            .waves
            .iter()
            .enumerate()
            .filter(|(_, w)| keep(w))
            .map(|(i, w)| AxisWave {
                target: w.target,
                phase: w.phase,
                wave: &layer.waves[i],
            })
            .collect()
    };
    let hue_axes = pick(&|w| {
        w.target == AnimTarget::Hue && w.phase != layer.params.color_phase
    });
    let bright_axes = pick(&|w| {
        w.target == AnimTarget::Brightness && w.phase != layer.params.color_phase
    });
    let warps = pick(&|w| !w.target.is_color());

    let field = PhaseField::of(&layer.params);
    vertex_pass(
        &mut layer.verts,
        &layer.mesh,
        VertexWork {
            field,
            base_spin: layer.params.spin as f32,
            warps: &warps,
            hue_axes: &hue_axes,
            bright_axes: &bright_axes,
            audio,
        },
    );
    draw_textured(
        &layer.mesh,
        Shaded {
            positions: &layer.verts.positions,
            uvs: &layer.verts.uvs,
            tints: Some(&layer.verts.tints),
        },
        field.wrap_period(),
        texture,
        base,
        sink,
    );
}

#[expect(unused)]
fn unused_shape_hint(_: &ShapeMesh) {}
