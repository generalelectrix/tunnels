use std::sync::Arc;

use crate::config::ClientConfig;
use crate::render::{RenderTarget, Vertex};
use lyon::math::{point, vector, Angle};
use lyon::tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use tunnels_lib::{RenderMode, Shape, Snapshot};

const TWOPI: f64 = 2.0 * PI;

pub type Color = [f32; 4];

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum TransformDirection {
    Vertical,
    Horizontal,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Transform {
    Flip(TransformDirection),
}

pub trait Draw {
    fn draw(&self, target: &mut dyn RenderTarget, cfg: &ClientConfig);
}

impl<T: Draw> Draw for Vec<T> {
    fn draw(&self, target: &mut dyn RenderTarget, cfg: &ClientConfig) {
        for e in self {
            e.draw(target, cfg);
        }
    }
}

impl<T: Draw> Draw for Arc<T> {
    fn draw(&self, target: &mut dyn RenderTarget, cfg: &ClientConfig) {
        (**self).draw(target, cfg);
    }
}

#[inline]
fn color_from_rgb(r: f64, g: f64, b: f64, a: f64) -> Color {
    [r as f32, g as f32, b as f32, a as f32]
}

#[inline]
fn hsv_to_rgb(hue: f64, sat: f64, val: f64, alpha: f64) -> Color {
    if sat == 0.0 {
        color_from_rgb(val, val, val, alpha)
    } else {
        let var_h = if hue == 1.0 { 0.0 } else { hue * 6.0 };

        let var_i = var_h.floor();
        let var_1 = val * (1.0 - sat);
        let var_2 = val * (1.0 - sat * (var_h - var_i));
        let var_3 = val * (1.0 - sat * (1.0 - (var_h - var_i)));

        match var_i as i64 {
            0 => color_from_rgb(val, var_3, var_1, alpha),
            1 => color_from_rgb(var_2, val, var_1, alpha),
            2 => color_from_rgb(var_1, val, var_3, alpha),
            3 => color_from_rgb(var_1, var_2, val, alpha),
            4 => color_from_rgb(var_3, var_1, val, alpha),
            _ => color_from_rgb(val, var_1, var_2, alpha),
        }
    }
}

/// Custom 2x3 row-major affine transformation matrix for 2D coordinate transforms.
///
/// `a.then(b)` means "apply a first, then b". `apply(x, y)` returns the transformed point.
struct Transform2D {
    // Row-major 2x3 affine matrix: [a, b, c, d, tx, ty]
    // [a  b  tx]   [x]
    // [c  d  ty] * [y]
    //              [1]
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Transform2D {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    fn translate(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx,
            ty,
        }
    }

    fn rotate(angle: f64) -> Self {
        let cos = angle.cos();
        let sin = angle.sin();
        Self {
            a: cos,
            b: -sin,
            c: sin,
            d: cos,
            tx: 0.0,
            ty: 0.0,
        }
    }

    fn scale(sx: f64, sy: f64) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: 0.0,
            ty: 0.0,
        }
    }

    fn then(&self, other: &Transform2D) -> Transform2D {
        // self applied first, then other: other * self
        Transform2D {
            a: other.a * self.a + other.b * self.c,
            b: other.a * self.b + other.b * self.d,
            c: other.c * self.a + other.d * self.c,
            d: other.c * self.b + other.d * self.d,
            tx: other.a * self.tx + other.b * self.ty + other.tx,
            ty: other.c * self.tx + other.d * self.ty + other.ty,
        }
    }

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.b * y + self.tx,
            self.c * x + self.d * y + self.ty,
        )
    }
}

/// Builds the full transform chain for a shape: rotate by `rot_angle * 2pi`, then flip if
/// configured, then translate to pixel position. This matches piston's original transform order.
fn build_transform(shape: &Shape, cfg: &ClientConfig) -> Transform2D {
    let (x, y) = {
        let (x0, y0) = match cfg.transformation {
            None => (shape.x, shape.y),
            Some(Transform::Flip(TransformDirection::Horizontal)) => (-shape.x, shape.y),
            Some(Transform::Flip(TransformDirection::Vertical)) => (shape.x, -shape.y),
        };
        let x = x0 * f64::from(cfg.x_resolution) + cfg.x_center;
        let y = y0 * f64::from(cfg.y_resolution) + cfg.y_center;
        (x, y)
    };

    let flip = match cfg.transformation {
        None => Transform2D::identity(),
        Some(Transform::Flip(TransformDirection::Horizontal)) => Transform2D::scale(-1.0, 1.0),
        Some(Transform::Flip(TransformDirection::Vertical)) => Transform2D::scale(1.0, -1.0),
    };

    // Order: rotate local geometry, then flip, then translate to pixel position
    Transform2D::rotate(shape.rot_angle * TWOPI)
        .then(&flip)
        .then(&Transform2D::translate(x, y))
}

/// Transforms lyon tessellation output (positions only) into `Vertex` structs with color and
/// transformed coordinates, then emits to `RenderTarget`.
fn emit_triangles(
    geometry: &VertexBuffers<[f32; 2], u32>,
    color: Color,
    transform: &Transform2D,
    target: &mut dyn RenderTarget,
) {
    let vertices: Vec<Vertex> = geometry
        .vertices
        .iter()
        .map(|pos| {
            let (tx, ty) = transform.apply(pos[0] as f64, pos[1] as f64);
            Vertex {
                position: [tx as f32, ty as f32],
                color,
            }
        })
        .collect();
    target.draw_triangles(&vertices, &geometry.indices);
}

impl Draw for Shape {
    fn draw(&self, target: &mut dyn RenderTarget, cfg: &ClientConfig) {
        let color = hsv_to_rgb(self.hue, self.sat, self.val, self.level);
        let transform = build_transform(self, cfg);

        match self.render_mode {
            // Arc rendering uses a custom quad-strip instead of lyon's stroke tessellator.
            // Lyon produces tangent-perpendicular endpoint cuts, but tunnel segments require
            // perfectly radial edges (important when viewing from center). The quad-strip
            // matches piston's `with_arc_tri_list`. Resolution is 128 steps per full circle,
            // matching piston.
            RenderMode::Arc => {
                let half_thickness = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;
                let rx = self.rad_x * cfg.critical_size;
                let ry = self.rad_y * cfg.critical_size;
                let start = self.start * TWOPI;
                let stop = self.stop * TWOPI;
                let sweep = stop - start;

                // Outer and inner ellipse radii (matching piston's cw1/ch1, cw2/ch2)
                let outer_rx = rx + half_thickness;
                let outer_ry = ry + half_thickness;
                let inner_rx = rx - half_thickness;
                let inner_ry = ry - half_thickness;

                // Resolution: match piston's approach (lower bound of 128 steps per full circle)
                let resolution = 128.0;
                let max_seg_size = TWOPI / resolution;
                let n_quads = (sweep.abs() / max_seg_size).ceil() as usize;
                let seg_size = sweep / n_quads as f64;

                // Build quad-strip with radial edges
                let mut vertices = Vec::with_capacity((n_quads + 1) * 2);
                let mut indices = Vec::with_capacity(n_quads * 6);

                for i in 0..=n_quads {
                    let angle = start + i as f64 * seg_size;
                    let cos = angle.cos();
                    let sin = angle.sin();
                    let (ox, oy) = transform.apply(cos * outer_rx, sin * outer_ry);
                    let (ix, iy) = transform.apply(cos * inner_rx, sin * inner_ry);
                    vertices.push(Vertex {
                        position: [ox as f32, oy as f32],
                        color,
                    });
                    vertices.push(Vertex {
                        position: [ix as f32, iy as f32],
                        color,
                    });
                }

                for i in 0..n_quads {
                    let base = (i * 2) as u32;
                    // outer[i], inner[i], outer[i+1]
                    indices.push(base);
                    indices.push(base + 1);
                    indices.push(base + 2);
                    // inner[i], outer[i+1], inner[i+1]
                    indices.push(base + 1);
                    indices.push(base + 2);
                    indices.push(base + 3);
                }

                target.draw_triangles(&vertices, &indices);
            }
            RenderMode::Dot => {
                let dot_radius = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;
                let mid_angle = (self.start + self.stop) / 2.0 * TWOPI;
                let cx = self.rad_x * cfg.critical_size * mid_angle.cos();
                let cy = self.rad_y * cfg.critical_size * mid_angle.sin();

                let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
                let mut tessellator = FillTessellator::new();
                if let Err(e) = tessellator.tessellate_ellipse(
                    point(cx as f32, cy as f32),
                    vector(dot_radius as f32, dot_radius as f32),
                    Angle::zero(),
                    lyon::path::Winding::Positive,
                    &FillOptions::default(),
                    &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
                        vertex.position().to_array()
                    }),
                ) {
                    log::error!("Ellipse tessellation failed: {e:?}");
                    return;
                }

                emit_triangles(&geometry, color, &transform, target);
            }
            RenderMode::Saucer => {
                let mid_angle = (self.start + self.stop) / 2.0 * TWOPI;
                let rx = self.rad_x * cfg.critical_size;
                let ry = self.rad_y * cfg.critical_size;
                let cx = rx * mid_angle.cos();
                let cy = ry * mid_angle.sin();

                let start_rad = self.start * TWOPI;
                let stop_rad = self.stop * TWOPI;
                let p1x = rx * start_rad.cos();
                let p1y = ry * start_rad.sin();
                let p2x = rx * stop_rad.cos();
                let p2y = ry * stop_rad.sin();
                let chord_len = ((p2x - p1x).powi(2) + (p2y - p1y).powi(2)).sqrt() / 2.0;

                let minor_radius = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;

                let tangent_angle = (ry * mid_angle.cos()).atan2(-rx * mid_angle.sin());

                // Saucer transform: parent_transform * translate(cx, cy) * rotate(tangent + spin)
                let local_transform = Transform2D::rotate(tangent_angle + self.spin_angle * TWOPI)
                    .then(&Transform2D::translate(cx, cy))
                    .then(&transform);

                let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
                let mut tessellator = FillTessellator::new();
                if let Err(e) = tessellator.tessellate_ellipse(
                    point(0.0, 0.0),
                    vector(chord_len as f32, minor_radius as f32),
                    Angle::zero(),
                    lyon::path::Winding::Positive,
                    &FillOptions::default(),
                    &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
                        vertex.position().to_array()
                    }),
                ) {
                    log::error!("Ellipse tessellation failed: {e:?}");
                    return;
                }

                emit_triangles(&geometry, color, &local_transform, target);
            }
        }
    }
}

impl Draw for Snapshot {
    fn draw(&self, target: &mut dyn RenderTarget, cfg: &ClientConfig) {
        self.layers.draw(target, cfg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const EPSILON: f64 = 1e-10;

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < EPSILON
    }

    #[test]
    fn test_identity() {
        let (x, y) = Transform2D::identity().apply(5.0, 10.0);
        assert_eq!(x, 5.0);
        assert_eq!(y, 10.0);
    }

    #[test]
    fn test_translate() {
        let (x, y) = Transform2D::translate(10.0, 20.0).apply(0.0, 0.0);
        assert_eq!(x, 10.0);
        assert_eq!(y, 20.0);
    }

    #[test]
    fn test_rotate_90() {
        let (x, y) = Transform2D::rotate(PI / 2.0).apply(1.0, 0.0);
        assert!(approx_eq(x, 0.0), "x was {x}, expected 0.0");
        assert!(approx_eq(y, 1.0), "y was {y}, expected 1.0");
    }

    #[test]
    fn test_scale_flip() {
        let (x, y) = Transform2D::scale(-1.0, 1.0).apply(5.0, 3.0);
        assert_eq!(x, -5.0);
        assert_eq!(y, 3.0);
    }

    #[test]
    fn test_composition_order() {
        let rotate = Transform2D::rotate(PI / 4.0);
        let translate = Transform2D::translate(10.0, 0.0);

        let (x1, y1) = rotate.then(&translate).apply(1.0, 0.0);
        let (x2, y2) = translate.then(&rotate).apply(1.0, 0.0);

        // rotate-then-translate and translate-then-rotate produce different results
        assert!(
            !approx_eq(x1, x2) || !approx_eq(y1, y2),
            "Expected different results, got ({x1},{y1}) vs ({x2},{y2})"
        );
    }

    #[test]
    fn test_hsv_grayscale() {
        let [r, g, b, a] = hsv_to_rgb(0.5, 0.0, 0.7, 1.0);
        assert_eq!(r, 0.7_f64 as f32);
        assert_eq!(g, 0.7_f64 as f32);
        assert_eq!(b, 0.7_f64 as f32);
        assert_eq!(a, 1.0);
    }

    #[test]
    fn test_hsv_hue_wraparound() {
        let c0 = hsv_to_rgb(0.0, 1.0, 1.0, 1.0);
        let c1 = hsv_to_rgb(1.0, 1.0, 1.0, 1.0);
        assert_eq!(c0, c1);
    }

    #[test]
    fn test_hsv_black() {
        let [r, g, b, _a] = hsv_to_rgb(0.5, 1.0, 0.0, 1.0);
        assert_eq!(r, 0.0);
        assert_eq!(g, 0.0);
        assert_eq!(b, 0.0);
    }
}
