//! Software rasteriser for piston's `Graphics` trait, used for headless output.
//!
//! Vendored from `tunnelclient/tests/software_graphics`, which is in turn based
//! on graphics_buffer (MIT). Extended here with `tri_list_c` so per-vertex
//! color gradients can be checked without a display.

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

    /// Wrap an existing image, so a color ramp can be used as a texture.
    pub fn from_image(inner: RgbaImage) -> Self {
        RenderBuffer { inner }
    }

    /// Sample with repeating wrap and nearest filtering.
    ///
    /// Repeat is what lets a phase coordinate run past one cycle and keep
    /// indexing the ramp, which is how the cycle count stays out of the mesh.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let (w, h) = self.inner.dimensions();
        let x = (u.rem_euclid(1.0) * w as f32) as u32;
        let y = (v.rem_euclid(1.0) * h as f32) as u32;
        color_rgba_f32(*self.inner.get_pixel(x.min(w - 1), y.min(h - 1)))
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
        (color[0].clamp(0.0, 1.0) * 255.0) as u8,
        (color[1].clamp(0.0, 1.0) * 255.0) as u8,
        (color[2].clamp(0.0, 1.0) * 255.0) as u8,
        (color[3].clamp(0.0, 1.0) * 255.0) as u8,
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

/// Barycentric weights of a point with respect to a triangle.
///
/// Returns `None` when the point falls outside, or when the triangle is
/// degenerate and has no interior to shade.
/// A triangle's corners, paired with a per-corner attribute.
type Attributed<T> = ([[f32; 2]; 3], [T; 3]);

/// A textured triangle that also carries a per-corner tint.
type Tinted = (Attributed<[f32; 2]>, [[f32; 4]; 3]);

fn barycentric(tri: &[[f32; 2]], p: [f32; 2]) -> Option<[f32; 3]> {
    let (a, b, c) = (tri[0], tri[1], tri[2]);
    let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if det.abs() < f32::EPSILON {
        return None;
    }
    let l0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
    let l1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
    let l2 = 1.0 - l0 - l1;
    // A small negative tolerance keeps adjacent triangles from leaving seams.
    const EDGE: f32 = -1e-4;
    (l0 >= EDGE && l1 >= EDGE && l2 >= EDGE).then_some([l0, l1, l2])
}

impl RenderBuffer {
    /// Rasterise one triangle, asking `shade` for the color at each covered
    /// pixel given its barycentric weights.
    fn raster(&mut self, tri: &[[f32; 2]], mut shade: impl FnMut([f32; 3]) -> [f32; 4]) {
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
                let Some(w) = barycentric(tri, [x as f32, y as f32]) else {
                    continue;
                };
                let over = shade(w);
                let under = color_rgba_f32(*self.inner.get_pixel(x as u32, y as u32));
                let blended = layer_color(&over, &under);
                self.inner
                    .put_pixel(x as u32, y as u32, color_f32_rgba(&blended));
            }
        }
    }
}

impl RenderBuffer {
    /// Rasterise one textured triangle, interpolating texture coordinates and
    /// sampling per pixel.
    fn raster_uv(
        &mut self,
        tri: &[[f32; 2]; 3],
        uv: &[[f32; 2]; 3],
        texture: &RenderBuffer,
        tint: [f32; 4],
    ) {
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
                let Some(w) = barycentric(tri, [x as f32, y as f32]) else {
                    continue;
                };
                let u = w[0] * uv[0][0] + w[1] * uv[1][0] + w[2] * uv[2][0];
                let v = w[0] * uv[0][1] + w[1] * uv[1][1] + w[2] * uv[2][1];
                let mut over = texture.sample(u, v);
                for ch in 0..4 {
                    over[ch] *= tint[ch];
                }
                let under = color_rgba_f32(*self.inner.get_pixel(x as u32, y as u32));
                let blended = layer_color(&over, &under);
                self.inner
                    .put_pixel(x as u32, y as u32, color_f32_rgba(&blended));
            }
        }
    }
}

impl RenderBuffer {
    /// Rasterise one textured triangle whose tint varies per vertex.
    ///
    /// This is the second colour channel: the texture carries whatever is
    /// indexed by the ramp coordinate, and the vertex tint carries something
    /// indexed by a different coordinate entirely.
    fn raster_uv_c(
        &mut self,
        tri: &[[f32; 2]; 3],
        uv: &[[f32; 2]; 3],
        cols: &[[f32; 4]; 3],
        texture: &RenderBuffer,
    ) {
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
                let Some(w) = barycentric(tri, [x as f32, y as f32]) else {
                    continue;
                };
                let u = w[0] * uv[0][0] + w[1] * uv[1][0] + w[2] * uv[2][0];
                let v = w[0] * uv[0][1] + w[1] * uv[1][1] + w[2] * uv[2][1];
                let mut over = texture.sample(u, v);
                for ch in 0..4 {
                    over[ch] *= w[0] * cols[0][ch] + w[1] * cols[1][ch] + w[2] * cols[2][ch];
                }
                let under = color_rgba_f32(*self.inner.get_pixel(x as u32, y as u32));
                let blended = layer_color(&over, &under);
                self.inner
                    .put_pixel(x as u32, y as u32, color_f32_rgba(&blended));
            }
        }
    }
}

impl Graphics for RenderBuffer {
    type Texture = RenderBuffer;

    fn clear_color(&mut self, color: Color) {
        let px = color_f32_rgba(&color);
        for pixel in self.inner.pixels_mut() {
            *pixel = px;
        }
    }

    fn clear_stencil(&mut self, _value: u8) {}

    fn tri_list<F>(&mut self, _draw_state: &DrawState, color: &[f32; 4], mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]])),
    {
        let color = *color;
        let mut tris: Vec<[[f32; 2]; 3]> = Vec::new();
        f(&mut |vertices| {
            for tri in vertices.chunks(3) {
                if let [a, b, c] = tri {
                    tris.push([*a, *b, *c]);
                }
            }
        });
        for tri in tris {
            self.raster(&tri, |_| color);
        }
    }

    fn tri_list_c<F>(&mut self, _draw_state: &DrawState, mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 4]])),
    {
        let mut tris: Vec<Attributed<[f32; 4]>> = Vec::new();
        f(&mut |vertices, colors| {
            for (v, c) in vertices.chunks(3).zip(colors.chunks(3)) {
                if let ([a, b, cc], [ca, cb, ccc]) = (v, c) {
                    tris.push(([*a, *b, *cc], [*ca, *cb, *ccc]));
                }
            }
        });
        for (tri, cols) in tris {
            self.raster(&tri, |w| {
                let mut out = [0.0; 4];
                for ch in 0..4 {
                    out[ch] = w[0] * cols[0][ch] + w[1] * cols[1][ch] + w[2] * cols[2][ch];
                }
                out
            });
        }
    }

    fn tri_list_uv<F>(
        &mut self,
        _draw_state: &DrawState,
        color: &[f32; 4],
        texture: &Self::Texture,
        mut f: F,
    ) where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]])),
    {
        let tint = *color;
        let mut tris: Vec<Attributed<[f32; 2]>> = Vec::new();
        f(&mut |vertices, coords| {
            for (v, t) in vertices.chunks(3).zip(coords.chunks(3)) {
                if let ([a, b, c], [ta, tb, tc]) = (v, t) {
                    tris.push(([*a, *b, *c], [*ta, *tb, *tc]));
                }
            }
        });
        for (tri, uv) in tris {
            self.raster_uv(&tri, &uv, texture, tint);
        }
    }

    fn tri_list_uv_c<F>(&mut self, _draw_state: &DrawState, texture: &Self::Texture, mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 2]], &[[f32; 4]])),
    {
        let mut tris: Vec<Tinted> = Vec::new();
        f(&mut |vertices, coords, colors| {
            for ((v, t), c) in vertices
                .chunks(3)
                .zip(coords.chunks(3))
                .zip(colors.chunks(3))
            {
                if let (([a, b, cc], [ta, tb, tc]), [ca, cb, ccc]) = ((v, t), c) {
                    tris.push((([*a, *b, *cc], [*ta, *tb, *tc]), [*ca, *cb, *ccc]));
                }
            }
        });
        for ((tri, uv), cols) in tris {
            self.raster_uv_c(&tri, &uv, &cols, texture);
        }
    }
}
