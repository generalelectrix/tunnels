//! The render window: the same piston/OpenGL stack `tunnelclient` uses, so what
//! shows up here is what the real client would produce.

use crate::draw::{draw_layer, draw_shaded, is_uniform, layer_transform, shade_mesh};
use crate::params::{DemoParams, LayerParams, PORT};
use crate::shapes::{ShapeMesh, load_dir};
use anyhow::{Result, anyhow};
use graphics::{Transformed, clear};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::net::UdpSocket;
use std::path::Path;
use std::time::Instant;

/// The parameters a layer's shaded mesh depends on.
///
/// Everything else — position, rotation, scale, shear — is a transform applied
/// to the finished mesh, so spinning a shape costs nothing beyond the
/// transform. Only these force a re-subdivide.
#[derive(PartialEq)]
struct MeshKey {
    shape: usize,
    outline: bool,
    stroke_width: u32,
    color: [u64; 5],
    phase: u8,
}

impl MeshKey {
    fn of(layer: &LayerParams) -> Self {
        Self {
            shape: layer.shape,
            outline: layer.draw_mode.draws_outline(),
            // Quantised so dragging a slider does not re-tessellate on every
            // sub-pixel step.
            stroke_width: (layer.stroke_width * 2000.0) as u32,
            color: [
                layer.col_center,
                layer.col_width,
                layer.col_spread,
                layer.col_sat,
                layer.level,
            ]
            .map(f64::to_bits),
            phase: layer.color_phase as u8,
        }
    }
}

/// A layer's shaded mesh in shape space, rebuilt only when `MeshKey` changes.
struct LayerCache {
    key: Option<MeshKey>,
    stroke: Vec<[f32; 2]>,
    fill_pos: Vec<[f32; 2]>,
    fill_col: Vec<[f32; 4]>,
    stroke_pos: Vec<[f32; 2]>,
    stroke_col: Vec<[f32; 4]>,
}

impl LayerCache {
    fn new() -> Self {
        Self {
            key: None,
            stroke: Vec::new(),
            fill_pos: Vec::new(),
            fill_col: Vec::new(),
            stroke_pos: Vec::new(),
            stroke_col: Vec::new(),
        }
    }

    fn refresh(&mut self, shapes: &[ShapeMesh], layer: &LayerParams) {
        let key = MeshKey::of(layer);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let shape = &shapes[layer.shape];
        self.stroke = if layer.draw_mode.draws_outline() {
            shape.stroke(layer.stroke_width as f32)
        } else {
            Vec::new()
        };
        let (fp, fc) = shade_mesh(&shape.fill, layer);
        let (sp, sc) = shade_mesh(&self.stroke, layer);
        self.fill_pos = fp;
        self.fill_col = fc;
        self.stroke_pos = sp;
        self.stroke_col = sc;
        self.key = Some(key);
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
    let mut caches: Vec<LayerCache> = (0..params.layers.len()).map(|_| LayerCache::new()).collect();
    let start = Instant::now();
    let mut buf = vec![0u8; 65536];

    while let Some(e) = window.next() {
        // Take the newest parameters waiting on the socket, dropping any
        // backlog — only the latest frame of control state matters.
        while let Ok(n) = socket.recv(&mut buf) {
            if let Ok(p) = serde_json::from_slice::<DemoParams>(&buf[..n]) {
                if p.layers.len() > caches.len() {
                    caches.resize_with(p.layers.len(), LayerCache::new);
                }
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
            for (layer, cache) in params.layers.iter().zip(caches.iter_mut()) {
                if !layer.enabled || layer.shape >= shapes.len() {
                    continue;
                }
                cache.refresh(&shapes, layer);
                let m = layer_transform(base, layer, time, critical);
                if is_uniform(layer) {
                    // A flat layer never needed subdividing, so draw it straight
                    // from the source mesh.
                    let outline = (!cache.stroke.is_empty()).then_some(cache.stroke.as_slice());
                    draw_layer(&shapes[layer.shape], outline, layer, m, gl);
                } else {
                    if layer.draw_mode.draws_fill() {
                        draw_shaded(&cache.fill_pos, &cache.fill_col, m, gl);
                    }
                    if layer.draw_mode.draws_outline() {
                        draw_shaded(&cache.stroke_pos, &cache.stroke_col, m, gl);
                    }
                }
            }
        });
    }
    Ok(())
}
