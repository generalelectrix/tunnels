//! The render window: the same piston/OpenGL stack `tunnelclient` uses, so what
//! shows up here is what the real client would produce.

use crate::draw::{color_mesh, draw_layer, draw_mesh, is_uniform, layer_transform};
use crate::mesh::{Level, MeshId, MeshLibrary};
use crate::params::{DemoParams, PORT};
use crate::shapes::{ShapeMesh, load_dir};
use anyhow::{Result, anyhow};
use graphics::{Transformed, clear};
use opengl_graphics::{GlGraphics, OpenGL};
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
    let mut strokes: Vec<StrokeCache> = (0..params.layers.len()).map(|_| StrokeCache::new()).collect();
    let mut meshes = MeshLibrary::default();
    let start = Instant::now();
    let mut buf = vec![0u8; 65536];
    let mut reported_meshes = 0;

    while let Some(e) = window.next() {
        // Take the newest parameters waiting on the socket, dropping any
        // backlog — only the latest frame of control state matters.
        while let Ok(n) = socket.recv(&mut buf) {
            if let Ok(p) = serde_json::from_slice::<DemoParams>(&buf[..n]) {
                if p.layers.len() > strokes.len() {
                    strokes.resize_with(p.layers.len(), StrokeCache::new);
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
            for (layer, stroke_cache) in params.layers.iter().zip(strokes.iter_mut()) {
                if !layer.enabled || layer.shape >= shapes.len() {
                    continue;
                }
                let m = layer_transform(base, layer, time, critical);
                let wants_outline = layer.draw_mode.draws_outline();
                let stroke = wants_outline
                    .then(|| stroke_cache.get(&shapes, layer.shape, layer.stroke_width).to_vec());

                // A flat or masked layer has nothing to interpolate across a
                // triangle, so it draws straight from the source mesh.
                if is_uniform(layer) || layer.mask {
                    draw_layer(&shapes[layer.shape], stroke.as_deref(), layer, m, gl);
                    continue;
                }

                let scale = layer.scale_x.abs().max(layer.scale_y.abs());
                let level = Level::for_scale(scale, critical);
                if layer.draw_mode.draws_fill() {
                    let id = MeshId {
                        shape: layer.shape,
                        stroke_width: None,
                        level,
                    };
                    let mesh = meshes.get(id, &shapes[layer.shape].fill);
                    let colors = color_mesh(mesh, layer);
                    draw_mesh(mesh, &colors, m, gl);
                }
                if let Some(stroke) = &stroke {
                    let id = MeshId {
                        shape: layer.shape,
                        stroke_width: Some((layer.stroke_width * 2000.0) as u32),
                        level,
                    };
                    let mesh = meshes.get(id, stroke);
                    let colors = color_mesh(mesh, layer);
                    draw_mesh(mesh, &colors, m, gl);
                }
            }
        });

        // Meshes are built the first time a shape is drawn at a given size, so
        // say so — a hitch on first use is expected, a hitch later is not.
        if meshes.len() != reported_meshes {
            reported_meshes = meshes.len();
            println!(
                "mesh library: {reported_meshes} meshes, {} triangles",
                meshes.triangles()
            );
        }
    }
    Ok(())
}
