//! Vendored software rasterizer for Piston's Graphics trait.
//! Based on graphics_buffer (MIT license, https://github.com/kaikalii/graphics_buffer).
//! Simplified: no rayon, no glyphs — flat, per-vertex-colored and textured
//! triangle rasterization.

use graphics::draw_state::DrawState;
use graphics::types::Color;
use graphics::{Graphics, ImageSize};
use image::{Rgba, RgbaImage};
use texture::{CreateTexture, Format, TextureOp, TextureSettings, UpdateTexture};

pub struct RenderBuffer {
    inner: RgbaImage,
}

impl RenderBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        RenderBuffer {
            inner: RgbaImage::new(width, height),
        }
    }

    /// Wrap an existing image, so a color ramp can serve as a texture.
    pub fn from_image(inner: RgbaImage) -> Self {
        RenderBuffer { inner }
    }

    pub fn into_image(self) -> RgbaImage {
        self.inner
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
    // A triangle with no signed area encloses nothing, and the sign tests
    // below cannot say so: all three collapse to zero, every one of them
    // satisfies `<=`, and the triangle then claims every point put to it —
    // its whole bounding box rather than the nothing it covers. Answering
    // that here is what makes a collapsed triangle draw nothing, the way a
    // hardware rasteriser and `barycentric` both already do.
    if sign(tri[0], tri[1], tri[2]) == 0.0 {
        return false;
    }
    // Use <= (inclusive edges) to avoid gaps between adjacent triangles.
    // This matches hardware rasterizer behavior more closely than strict <.
    // Edge pixels may be claimed by both adjacent triangles, but since all
    // triangles in a shape share the same color, the double-draw is invisible.
    let b1 = sign(point, tri[0], tri[1]) <= 0.0;
    let b2 = sign(point, tri[1], tri[2]) <= 0.0;
    let b3 = sign(point, tri[2], tri[0]) <= 0.0;
    (b1 && b2 && b3) || (!b1 && !b2 && !b3)
}

/// A triangle's corners, paired with a per-corner attribute.
type Attributed<T> = ([[f32; 2]; 3], [T; 3]);

/// A textured triangle that also carries a per-corner tint.
type Tinted = (Attributed<[f32; 2]>, [[f32; 4]; 3]);

/// Barycentric weights of a point with respect to a triangle.
///
/// Returns `None` when the point falls outside, or when the triangle is
/// degenerate and has no interior to shade.
fn barycentric(tri: &[[f32; 2]; 3], p: [f32; 2]) -> Option<[f32; 3]> {
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

/// Interpolate a per-corner attribute at the given barycentric weights.
fn interpolate<const N: usize>(w: [f32; 3], corners: &[[f32; N]; 3]) -> [f32; N] {
    std::array::from_fn(|ch| w[0] * corners[0][ch] + w[1] * corners[1][ch] + w[2] * corners[2][ch])
}

impl RenderBuffer {
    /// Rasterise one triangle, asking `shade` for the color at each covered
    /// pixel given its barycentric weights.
    ///
    /// Kept apart from `tri_list`'s own loop: that one decides coverage by
    /// signed area, and the golden images rest on exactly which edge pixels it
    /// claims.
    fn raster(&mut self, tri: &[[f32; 2]; 3], mut shade: impl FnMut([f32; 3]) -> [f32; 4]) {
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

impl tunnelclient::fill::gpu::FillBackend for RenderBuffer {}

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
            self.raster(&tri, |w| {
                let [u, v] = interpolate(w, &uv);
                let mut over = texture.sample(u, v);
                for (ch, t) in over.iter_mut().zip(tint) {
                    *ch *= t;
                }
                over
            });
        }
    }

    fn tri_list_c<F>(&mut self, _draw_state: &DrawState, mut f: F)
    where
        F: FnMut(&mut dyn FnMut(&[[f32; 2]], &[[f32; 4]])),
    {
        let mut tris: Vec<Attributed<[f32; 4]>> = Vec::new();
        f(&mut |vertices, colors| {
            for (v, c) in vertices.chunks(3).zip(colors.chunks(3)) {
                if let ([a, b, c2], [ca, cb, cc]) = (v, c) {
                    tris.push(([*a, *b, *c2], [*ca, *cb, *cc]));
                }
            }
        });
        for (tri, cols) in tris {
            self.raster(&tri, |w| interpolate(w, &cols));
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
                if let (([a, b, c2], [ta, tb, tc]), [ca, cb, cc]) = ((v, t), c) {
                    tris.push((([*a, *b, *c2], [*ta, *tb, *tc]), [*ca, *cb, *cc]));
                }
            }
        });
        for ((tri, uv), cols) in tris {
            self.raster(&tri, |w| {
                let [u, v] = interpolate(w, &uv);
                let mut over = texture.sample(u, v);
                for (ch, t) in over.iter_mut().zip(interpolate(w, &cols)) {
                    *ch *= t;
                }
                over
            });
        }
    }
}

impl TextureOp<()> for RenderBuffer {
    type Error = String;
}

impl CreateTexture<()> for RenderBuffer {
    fn create<S: Into<[u32; 2]>>(
        _factory: &mut (),
        _format: Format,
        memory: &[u8],
        size: S,
        _settings: &TextureSettings,
    ) -> Result<Self, Self::Error> {
        let [width, height] = size.into();
        RgbaImage::from_raw(width, height, memory.to_vec())
            .map(RenderBuffer::from_image)
            .ok_or_else(|| {
                format!(
                    "{} bytes is not a {width}x{height} RGBA image",
                    memory.len()
                )
            })
    }
}

impl UpdateTexture<()> for RenderBuffer {
    fn update<O, S>(
        &mut self,
        _factory: &mut (),
        _format: Format,
        memory: &[u8],
        offset: O,
        size: S,
    ) -> Result<(), Self::Error>
    where
        O: Into<[u32; 2]>,
        S: Into<[u32; 2]>,
    {
        let [x, y] = offset.into();
        let [width, height] = size.into();
        let expected = (width as usize) * (height as usize) * 4;
        if memory.len() < expected {
            return Err(format!(
                "{} bytes is short of the {expected} a {width}x{height} update needs",
                memory.len()
            ));
        }
        for row in 0..height {
            for col in 0..width {
                let i = ((row * width + col) * 4) as usize;
                let px = Rgba([memory[i], memory[i + 1], memory[i + 2], memory[i + 3]]);
                self.inner.put_pixel(x + col, y + row, px);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// How many pixels a run of triangles leaves lit on a cleared buffer.
    fn lit(triangles: &[[f32; 2]]) -> usize {
        let mut buffer = RenderBuffer::new(16, 16);
        buffer.clear_color([0.0, 0.0, 0.0, 1.0]);
        buffer.tri_list(&DrawState::default(), &[1.0, 1.0, 1.0, 1.0], |f| {
            f(triangles);
        });
        buffer
            .into_image()
            .pixels()
            .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
            .count()
    }

    /// A triangle with no area covers no pixels.
    ///
    /// Both ways of having none are drawn, because they reach the sign tests
    /// differently: three coincident corners leave every test at zero, so the
    /// triangle claims whatever is put to it, while three collinear ones leave
    /// them disagreeing off the line and agreeing along it. A stroke
    /// contracted to nothing is made of the first kind — every vertex of a
    /// round join's fan shares one contour point — so a rasteriser that draws
    /// it paints a dotted contour where the figure has vanished.
    ///
    /// The coincident corners sit off the pixel grid, because a bounding box
    /// taken between the floor and the ceiling of one integer coordinate is
    /// empty and would leave that case untested.
    #[test]
    fn a_triangle_with_no_area_covers_nothing() {
        assert_eq!(
            lit(&[[4.5, 4.5], [4.5, 4.5], [4.5, 4.5]]),
            0,
            "three coincident corners lit pixels"
        );
        assert_eq!(
            lit(&[[2.0, 2.0], [7.0, 7.0], [12.0, 12.0]]),
            0,
            "three collinear corners lit pixels"
        );
        // The same triangle with one corner moved off the line does cover
        // pixels, so the two above are empty for want of area and not for want
        // of a working rasteriser.
        assert!(
            lit(&[[2.0, 2.0], [12.0, 2.0], [12.0, 12.0]]) > 0,
            "a triangle with area lit nothing"
        );
    }
}
