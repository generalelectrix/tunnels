//! Vendored software rasterizer for Piston's Graphics trait.
//! Based on graphics_buffer (MIT license, https://github.com/kaikalii/graphics_buffer).
//! Simplified: no rayon, no texture UV, no glyphs — just solid-color triangle rasterization.

use graphics::draw_state::DrawState;
use graphics::types::Color;
use graphics::{Graphics, ImageSize};
use image::{Rgba, RgbaImage};

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

impl ImageSize for RenderBuffer {
    fn get_size(&self) -> (u32, u32) {
        self.inner.dimensions()
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

fn triangle_contains(tri: &[[f32; 2]], point: [f32; 2]) -> bool {
    let b1 = sign(point, tri[0], tri[1]) < 0.0;
    let b2 = sign(point, tri[1], tri[2]) < 0.0;
    let b3 = sign(point, tri[2], tri[0]) < 0.0;
    b1 == b2 && b2 == b3
}

impl Graphics for RenderBuffer {
    type Texture = RenderBuffer;

    fn clear_color(&mut self, color: Color) {
        for (_, _, pixel) in self.inner.enumerate_pixels_mut() {
            *pixel = color_f32_rgba(&color);
        }
    }

    fn clear_stencil(&mut self, _value: u8) {}

    fn tri_list<F>(&mut self, _draw_state: &DrawState, color: &[f32; 4], mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]])),
    {
        f(&mut |vertices| {
            for tri in vertices.chunks(3) {
                if tri.len() < 3 {
                    continue;
                }
                // Bounding box
                let mut tl = [f32::MAX, f32::MAX];
                let mut br = [f32::MIN, f32::MIN];
                for v in tri {
                    tl[0] = tl[0].min(v[0]);
                    tl[1] = tl[1].min(v[1]);
                    br[0] = br[0].max(v[0]);
                    br[1] = br[1].max(v[1]);
                }
                let x0 = tl[0].floor().max(0.0) as i32;
                let y0 = tl[1].floor().max(0.0) as i32;
                let x1 = br[0].ceil().min(self.inner.width() as f32) as i32;
                let y1 = br[1].ceil().min(self.inner.height() as f32) as i32;

                for x in x0..x1 {
                    for y in y0..y1 {
                        if triangle_contains(tri, [x as f32, y as f32]) {
                            let under = color_rgba_f32(*self.inner.get_pixel(x as u32, y as u32));
                            let blended = layer_color(color, &under);
                            self.inner
                                .put_pixel(x as u32, y as u32, color_f32_rgba(&blended));
                        }
                    }
                }
            }
        });
    }

    fn tri_list_uv<F>(
        &mut self,
        _draw_state: &DrawState,
        _color: &[f32; 4],
        _texture: &Self::Texture,
        _f: F,
    ) where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]])),
    {
        unimplemented!("tri_list_uv not needed for arc rendering tests")
    }

    fn tri_list_c<F>(&mut self, _: &DrawState, _: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 4]])),
    {
        unimplemented!("tri_list_c not needed for arc rendering tests")
    }

    fn tri_list_uv_c<F>(&mut self, _: &DrawState, _: &Self::Texture, _: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]], &[[f32; 4]])),
    {
        unimplemented!("tri_list_uv_c not needed for arc rendering tests")
    }
}
