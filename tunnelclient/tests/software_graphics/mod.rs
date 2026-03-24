//! Software rasterizer implementing the RenderTarget trait.
//! Based on the previous piston Graphics trait implementation.
//! Simplified: just solid-color indexed triangle rasterization with alpha blending.

use image::{Rgba, RgbaImage};
use tunnelclient::render::{RenderTarget, Vertex};

pub struct RenderBuffer {
    inner: RgbaImage,
}

impl RenderBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        RenderBuffer {
            inner: RgbaImage::new(width, height),
        }
    }

    pub fn into_image(self) -> RgbaImage {
        self.inner
    }
}

fn color_f32_rgba(color: &[f32; 4]) -> Rgba<u8> {
    Rgba([
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    ])
}

fn color_rgba_f32(color: Rgba<u8>) -> [f32; 4] {
    [
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    ]
}

fn layer_color(over: &[f32; 4], under: &[f32; 4]) -> [f32; 4] {
    let over_weight = 1.0 - (1.0 - over[3]).powf(2.0);
    let under_weight = 1.0 - over_weight;
    [
        over_weight * over[0] + under_weight * under[0],
        over_weight * over[1] + under_weight * under[1],
        over_weight * over[2] + under_weight * under[2],
        (over[3].powf(2.0) + under[3].powf(2.0)).sqrt().min(1.0),
    ]
}

fn sign(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> f32 {
    (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])
}

fn triangle_contains(v0: [f32; 2], v1: [f32; 2], v2: [f32; 2], point: [f32; 2]) -> bool {
    // Use <= (inclusive edges) to avoid gaps between adjacent triangles.
    // This matches hardware rasterizer behavior more closely than strict <.
    // Edge pixels may be claimed by both adjacent triangles, but since all
    // triangles in a shape share the same color, the double-draw is invisible.
    let b1 = sign(point, v0, v1) <= 0.0;
    let b2 = sign(point, v1, v2) <= 0.0;
    let b3 = sign(point, v2, v0) <= 0.0;
    (b1 && b2 && b3) || (!b1 && !b2 && !b3)
}

impl RenderTarget for RenderBuffer {
    fn clear(&mut self, color: [f32; 4]) {
        for (_, _, pixel) in self.inner.enumerate_pixels_mut() {
            *pixel = color_f32_rgba(&color);
        }
    }

    fn draw_triangles(&mut self, vertices: &[Vertex], indices: &[u32]) {
        for tri in indices.chunks(3) {
            if tri.len() < 3 {
                continue;
            }
            let v0 = vertices[tri[0] as usize];
            let v1 = vertices[tri[1] as usize];
            let v2 = vertices[tri[2] as usize];
            let color = v0.color;

            // Bounding box
            let mut tl = [f32::MAX, f32::MAX];
            let mut br = [f32::MIN, f32::MIN];
            for pos in [v0.position, v1.position, v2.position] {
                tl[0] = tl[0].min(pos[0]);
                tl[1] = tl[1].min(pos[1]);
                br[0] = br[0].max(pos[0]);
                br[1] = br[1].max(pos[1]);
            }
            let x0 = tl[0].floor().max(0.0) as i32;
            let y0 = tl[1].floor().max(0.0) as i32;
            let x1 = br[0].ceil().min(self.inner.width() as f32) as i32;
            let y1 = br[1].ceil().min(self.inner.height() as f32) as i32;

            for x in x0..x1 {
                for y in y0..y1 {
                    if triangle_contains(
                        v0.position,
                        v1.position,
                        v2.position,
                        [x as f32, y as f32],
                    ) {
                        let under = color_rgba_f32(*self.inner.get_pixel(x as u32, y as u32));
                        let blended = layer_color(&color, &under);
                        self.inner
                            .put_pixel(x as u32, y as u32, color_f32_rgba(&blended));
                    }
                }
            }
        }
    }
}
