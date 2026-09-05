//! The render window: the same piston/OpenGL stack `tunnelclient` uses, so what
//! shows up here is what the real client would produce.

use crate::draw::{draw_layer, layer_transform};
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

/// Cached stroke geometry, rebuilt only when a layer's shape or width changes.
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

    /// Stroke triangles for a shape at a width, re-tessellating on change.
    ///
    /// The width is quantised into the key so that dragging a slider does not
    /// re-tessellate on every sub-pixel step.
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
    let mut caches: Vec<StrokeCache> = (0..params.layers.len()).map(|_| StrokeCache::new()).collect();
    let start = Instant::now();
    let mut buf = vec![0u8; 65536];

    while let Some(e) = window.next() {
        // Take the newest parameters waiting on the socket, dropping any
        // backlog — only the latest frame of control state matters.
        while let Ok(n) = socket.recv(&mut buf) {
            if let Ok(p) = serde_json::from_slice::<DemoParams>(&buf[..n]) {
                if p.layers.len() > caches.len() {
                    caches.resize_with(p.layers.len(), StrokeCache::new);
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
                let outline = layer
                    .draw_mode
                    .draws_outline()
                    .then(|| cache.get(&shapes, layer.shape, layer.stroke_width).to_vec());
                draw_layer(
                    &shapes[layer.shape],
                    outline.as_deref(),
                    layer,
                    layer_transform(base, layer, time, critical),
                    gl,
                );
            }
        });
    }
    Ok(())
}
