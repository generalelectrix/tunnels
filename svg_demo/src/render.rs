//! The render window: the same piston/OpenGL stack `tunnelclient` uses, so what
//! shows up here is what the real client would produce.

use crate::draw::{draw_layer, draw_textured, is_uniform, layer_transform, phase_uvs};
use crate::mesh::{Level, MeshId, MeshLibrary};
use crate::params::{DemoParams, LayerParams, PhaseField, PORT};
use crate::ramp::{self, RampKey};
use image::RgbaImage;
use crate::shapes::{ShapeMesh, load_dir};
use anyhow::{Result, anyhow};
use graphics::math::Matrix2d;
use graphics::{Graphics, Transformed, clear};
use opengl_graphics::{Filter, GlGraphics, OpenGL, Texture, TextureSettings, Wrap};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::net::UdpSocket;
use std::path::Path;
use std::time::Instant;

/// Stroke geometry for a layer, rebuilt only when its shape or width changes.
///
/// Kept apart from the mesh library because a stroke has to be tessellated
/// before it can be refined, and that tessellation does not depend on density.
struct StrokeCache {
    key: Option<(usize, u32)>,
    tris: Vec<[f32; 2]>,
}

impl StrokeCache {
    fn new() -> Self {
        Self {
            key: None,
            tris: Vec::new(),
        }
    }

    fn get(&mut self, shapes: &[ShapeMesh], shape: usize, width: f64) -> &[[f32; 2]] {
        let key = (shape, (width * 2000.0) as u32);
        if self.key != Some(key) {
            self.tris = shapes[shape].stroke(width as f32);
            self.key = Some(key);
        }
        &self.tris
    }
}

/// How many textures a layer rotates through when its ramp changes.
///
/// Overwriting a texture the GPU is still sampling makes the driver stall until
/// the queued draws that read it retire. Measured, that stall was two orders of
/// magnitude larger than building the ramp in the first place. Rotating means a
/// texture is only rewritten once the frames that used it are long done.
const RAMP_BUFFERS: usize = 3;

/// A layer's ramp textures and the parameters they were built from.
struct RampSlot {
    key: Option<RampKey>,
    textures: Vec<Texture>,
    current: usize,
    scratch: RgbaImage,
}

impl RampSlot {
    fn new(template: &LayerParams, settings: &TextureSettings) -> Self {
        let img = ramp::build(template);
        Self {
            key: None,
            textures: (0..RAMP_BUFFERS)
                .map(|_| Texture::from_image(&img, settings))
                .collect(),
            current: 0,
            scratch: img,
        }
    }

    /// Rebuild if the color knobs moved, and return the texture to sample.
    fn refresh(&mut self, layer: &LayerParams) -> &Texture {
        let key = RampKey::of(layer);
        if self.key != Some(key) {
            ramp::build_into(&mut self.scratch, layer);
            self.current = (self.current + 1) % self.textures.len();
            self.textures[self.current].update(&self.scratch);
            self.key = Some(key);
        }
        &self.textures[self.current]
    }
}

/// Where a frame's CPU time went.
///
/// Reported by the profiler so a regression can be attributed to a stage
/// rather than just showing up as a slower frame.
#[derive(Default, Clone, Copy)]
pub struct Timings {
    /// Rebuilding ramp textures, which happens only when a color knob moves.
    pub ramp_us: u128,
    /// Refining meshes, which happens only on a cache miss.
    pub mesh_us: u128,
    /// Evaluating phase per unique vertex. Paid every frame.
    pub uv_us: u128,
    /// Projecting vertices and filling the backend's buffers. Paid every frame.
    pub submit_us: u128,
    pub triangles: usize,
}

impl Timings {
    pub fn cpu_us(&self) -> u128 {
        self.ramp_us + self.mesh_us + self.uv_us + self.submit_us
    }
}

/// Everything the render window needs to draw a frame, and the caches behind it.
///
/// Shared by the live window and the profiler so both exercise the same path.
pub struct Renderer {
    pub shapes: Vec<ShapeMesh>,
    strokes: Vec<StrokeCache>,
    meshes: MeshLibrary,
    ramps: Vec<RampSlot>,
    ramp_settings: TextureSettings,
}

impl Renderer {
    pub fn new(shapes: Vec<ShapeMesh>) -> Self {
        // Repeating wrap is what lets a phase coordinate run past one cycle and
        // keep indexing the ramp, so the cycle count never reaches the mesh.
        // Linear filtering puts the sawtooth's jump inside a single texel.
        let ramp_settings = TextureSettings::new()
            .filter(Filter::Linear)
            .wrap_u(Wrap::Repeat)
            .wrap_v(Wrap::Repeat);
        Self {
            shapes,
            strokes: Vec::new(),
            meshes: MeshLibrary::default(),
            ramps: Vec::new(),
            ramp_settings,
        }
    }

    /// Grow the per-layer caches to cover this many layers.
    pub fn ensure_layers(&mut self, n: usize, template: &LayerParams) {
        while self.strokes.len() < n {
            self.strokes.push(StrokeCache::new());
        }
        while self.ramps.len() < n {
            self.ramps.push(RampSlot::new(template, &self.ramp_settings));
        }
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    pub fn mesh_triangles(&self) -> usize {
        self.meshes.triangles()
    }

    /// Draw every enabled layer, timing each stage.
    pub fn draw<G>(
        &mut self,
        gl: &mut G,
        base: Matrix2d,
        critical: f64,
        params: &DemoParams,
        time: f64,
    ) -> Timings
    where
        G: Graphics<Texture = Texture>,
    {
        let Self {
            shapes,
            strokes,
            meshes,
            ramps,
            ..
        } = self;
        let mut t = Timings::default();

        for ((layer, stroke_cache), ramp_slot) in params
            .layers
            .iter()
            .zip(strokes.iter_mut())
            .zip(ramps.iter_mut())
        {
            if !layer.enabled || layer.shape >= shapes.len() {
                continue;
            }
            let m = layer_transform(base, layer, time, critical);
            let stroke = layer
                .draw_mode
                .draws_outline()
                .then(|| stroke_cache.get(shapes, layer.shape, layer.stroke_width).to_vec());

            // A flat or masked layer has nothing to interpolate across a
            // triangle, so it draws straight from the source mesh.
            if is_uniform(layer) || layer.mask {
                let mark = Instant::now();
                draw_layer(&shapes[layer.shape], stroke.as_deref(), layer, m, gl);
                t.submit_us += mark.elapsed().as_micros();
                t.triangles += shapes[layer.shape].fill.len() / 3;
                continue;
            }

            // Rebuilding the ramp is a thousand-texel write, so a color knob
            // costs that and nothing else — the mesh never moves.
            let mark = Instant::now();
            let ramp_texture = ramp_slot.refresh(layer);
            t.ramp_us += mark.elapsed().as_micros();

            let scale = layer.scale_x.abs().max(layer.scale_y.abs());
            let level = Level::for_scale(scale, critical);
            let field = PhaseField::of(layer);

            let mut piece = |source: &[[f32; 2]], stroke_width: Option<u32>, t: &mut Timings| {
                let id = MeshId {
                    shape: layer.shape,
                    stroke_width,
                    level,
                };
                let mark = Instant::now();
                let mesh = meshes.get(id, source);
                t.mesh_us += mark.elapsed().as_micros();

                let mark = Instant::now();
                let uvs = phase_uvs(mesh, field);
                t.uv_us += mark.elapsed().as_micros();

                let mark = Instant::now();
                draw_textured(mesh, &uvs, ramp_texture, m, gl);
                t.submit_us += mark.elapsed().as_micros();
                t.triangles += mesh.triangle_count();
            };

            if layer.draw_mode.draws_fill() {
                piece(&shapes[layer.shape].fill, None, &mut t);
            }
            if let Some(stroke) = &stroke {
                piece(stroke, Some((layer.stroke_width * 2000.0) as u32), &mut t);
            }
        }
        t
    }
}

pub fn run(shape_dir: &Path) -> Result<()> {
    let shapes = load_dir(shape_dir)?;
    println!("render: {} shapes", shapes.len());

    let socket = UdpSocket::bind(("127.0.0.1", PORT))?;
    socket.set_nonblocking(true)?;

    let opengl = OpenGL::V3_2;
    let mut window: PistonWindow<Sdl2Window> =
        WindowSettings::new("svg_demo: render", [1280, 720])
            .graphics_api(opengl)
            .exit_on_esc(true)
            .vsync(true)
            .samples(4)
            .build()
            .map_err(|e| anyhow!("{e}"))?;
    let mut gl = GlGraphics::new(opengl);

    let mut params = DemoParams::default();
    let mut renderer = Renderer::new(shapes);
    renderer.ensure_layers(params.layers.len(), &params.layers[0]);
    let start = Instant::now();
    let mut buf = vec![0u8; 65536];
    let mut reported_meshes = 0;

    while let Some(e) = window.next() {
        // Take the newest parameters waiting on the socket, dropping any
        // backlog — only the latest frame of control state matters.
        while let Ok(n) = socket.recv(&mut buf) {
            if let Ok(p) = serde_json::from_slice::<DemoParams>(&buf[..n]) {
                renderer.ensure_layers(p.layers.len(), &p.layers[0]);
                params = p;
            }
        }

        let Some(args) = e.render_args() else { continue };
        let time = start.elapsed().as_secs_f64();
        let (w, h) = (args.window_size[0], args.window_size[1]);
        let critical = w.min(h);

        gl.draw(args.viewport(), |c, gl| {
            clear([0.0, 0.0, 0.0, 1.0], gl);
            let base = c.transform.trans(w / 2.0, h / 2.0);
            renderer.draw(gl, base, critical, &params, time);
        });

        // Meshes are built the first time a shape is drawn at a given size, so
        // say so — a hitch on first use is expected, a hitch later is not.
        if renderer.mesh_count() != reported_meshes {
            reported_meshes = renderer.mesh_count();
            println!(
                "mesh library: {reported_meshes} meshes, {} triangles",
                renderer.mesh_triangles()
            );
        }
    }
    Ok(())
}
