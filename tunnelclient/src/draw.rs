use std::sync::Arc;

use client_lib::config::ClientConfig;
use client_lib::transform::{Transform, TransformDirection};
use graphics::Context;
use graphics::types::Color;
use graphics::{CircleArc, Graphics, Transformed, ellipse, line, rectangle};
use std::f64::consts::PI;
use tunnels_lib::{PathShape, RenderMode, Shape, Snapshot};

const TWOPI: f64 = 2.0 * PI;

pub trait Draw<G: Graphics> {
    /// Given a context and gl instance, draw this entity to the screen.
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig);
}

impl<T, G> Draw<G> for Vec<T>
where
    G: Graphics,
    T: Draw<G>,
{
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        for e in self {
            e.draw(c, gl, cfg);
        }
    }
}

impl<T, G> Draw<G> for Arc<T>
where
    G: Graphics,
    T: Draw<G>,
{
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        (**self).draw(c, gl, cfg);
    }
}

#[inline]
fn color_from_rgb(r: f64, g: f64, b: f64, a: f64) -> Color {
    [r as f32, g as f32, b as f32, a as f32]
}

/// Convert HSV to a Piston RGB color.
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

impl<G: Graphics> Draw<G> for Shape {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        let color = hsv_to_rgb(self.hue, self.sat, self.val, self.level);
        let thickness = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;
        let spin_rad = self.spin_angle * TWOPI;

        let (x, y) = {
            let (x0, y0) = match cfg.transformation {
                None => (self.x, self.y),
                Some(Transform::Flip(TransformDirection::Horizontal)) => (-self.x, self.y),
                Some(Transform::Flip(TransformDirection::Vertical)) => (self.x, -self.y),
            };
            let x = x0 * f64::from(cfg.x_resolution) + cfg.x_center;
            let y = y0 * f64::from(cfg.y_resolution) + cfg.y_center;
            (x, y)
        };

        let transform = {
            let t = c.transform.trans(x, y);
            match cfg.transformation {
                None => t,
                Some(Transform::Flip(TransformDirection::Horizontal)) => t.flip_h(),
                Some(Transform::Flip(TransformDirection::Vertical)) => t.flip_v(),
            }
        }
        .rot_rad(self.rot_angle * TWOPI);

        match self.path_shape {
            PathShape::Ellipse => {
                draw_ellipse(self, color, thickness, spin_rad, transform, gl, cfg);
            }
            PathShape::Line => {
                draw_line(self, color, thickness, spin_rad, transform, gl, cfg);
            }
        }
    }
}

fn draw_ellipse<G: Graphics>(
    shape: &Shape,
    color: Color,
    thickness: f64,
    spin_rad: f64,
    transform: graphics::math::Matrix2d,
    gl: &mut G,
    cfg: &ClientConfig,
) {
    match shape.render_mode {
        RenderMode::Arc => {
            let x_size = shape.rad_x * cfg.critical_size;
            let y_size = shape.rad_y * cfg.critical_size;
            let start = shape.start * TWOPI;
            let stop = shape.stop * TWOPI;
            let bound = rectangle::centered([0.0, 0.0, x_size, y_size]);

            if stop - start >= TWOPI {
                // Full circle: no meaningful centroid to spin around.
                CircleArc::new(color, thickness, start, stop).draw(
                    bound,
                    &Default::default(),
                    transform,
                    gl,
                );
            } else {
                // Compute centroid of this arc segment on the ellipse.
                let mid_angle = (start + stop) / 2.0;
                let cx = x_size * mid_angle.cos();
                let cy = y_size * mid_angle.sin();

                // Draw the arc rotated around its centroid by spin_angle.
                // Offset the ellipse bound so the centroid is at the local origin,
                // apply spin rotation, then translate to the centroid position.
                let local = transform.trans(cx, cy).rot_rad(spin_rad);
                let offset_bound = rectangle::centered([-cx, -cy, x_size, y_size]);
                CircleArc::new(color, thickness, start, stop).draw(
                    offset_bound,
                    &Default::default(),
                    local,
                    gl,
                );
            }
        }
        RenderMode::Dot => {
            let mid_angle = (shape.start + shape.stop) / 2.0 * TWOPI;
            let cx = shape.rad_x * cfg.critical_size * mid_angle.cos();
            let cy = shape.rad_y * cfg.critical_size * mid_angle.sin();
            let bound = rectangle::centered([cx, cy, thickness, thickness]);
            ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
        }
        RenderMode::Saucer => {
            let mid_angle = (shape.start + shape.stop) / 2.0 * TWOPI;
            let rx = shape.rad_x * cfg.critical_size;
            let ry = shape.rad_y * cfg.critical_size;
            let cx = rx * mid_angle.cos();
            let cy = ry * mid_angle.sin();

            let start_rad = shape.start * TWOPI;
            let stop_rad = shape.stop * TWOPI;
            let p1x = rx * start_rad.cos();
            let p1y = ry * start_rad.sin();
            let p2x = rx * stop_rad.cos();
            let p2y = ry * stop_rad.sin();
            let chord_len = ((p2x - p1x).powi(2) + (p2y - p1y).powi(2)).sqrt() / 2.0;

            let tangent_angle = (ry * mid_angle.cos()).atan2(-rx * mid_angle.sin());

            let bound = rectangle::centered([0.0, 0.0, chord_len, thickness]);
            let local_transform = transform.trans(cx, cy).rot_rad(tangent_angle + spin_rad);
            ellipse::Ellipse::new(color).draw(bound, &Default::default(), local_transform, gl);
        }
    }
}

/// Wrapping and cross-fade geometry for a segment on a line path.
struct LineWrap {
    /// Position of the segment midpoint, wrapped into [left_end, right_end].
    mid: f64,
    /// Fade factor in [0, 1]. 1.0 = fully visible, <1.0 = near an endpoint.
    fade: f64,
    /// Mirror position at the opposite end, if fading.
    mirror_pos: Option<f64>,
}

fn compute_line_wrap(mid_pos: f64, left_end: f64, right_end: f64, fade_zone: f64) -> LineWrap {
    let line_len = right_end - left_end;

    let mid = if line_len > 0.0 {
        let shifted = mid_pos - left_end;
        left_end + ((shifted % line_len) + line_len) % line_len
    } else {
        0.0
    };

    let dist_to_edge = (mid - left_end).min(right_end - mid);
    let fade = if fade_zone > 0.0 {
        (dist_to_edge / fade_zone).min(1.0)
    } else {
        1.0
    };

    let mirror_pos = if fade < 1.0 {
        // Mirror appears at the opposite endpoint from whichever edge the
        // dot is fading near.
        Some(if mid - left_end < right_end - mid {
            right_end
        } else {
            left_end
        })
    } else {
        None
    };

    LineWrap {
        mid,
        fade,
        mirror_pos,
    }
}

fn draw_line<G: Graphics>(
    shape: &Shape,
    color: Color,
    thickness: f64,
    spin_rad: f64,
    transform: graphics::math::Matrix2d,
    gl: &mut G,
    cfg: &ClientConfig,
) {
    let half_length = shape.rad_x * cfg.critical_size;
    let y_offset = shape.rad_y * cfg.critical_size;

    // Normalize start/stop to [0, 1) and compute segment span.
    let start_norm = ((shape.start % 1.0) + 1.0) % 1.0;
    let seg_width = shape.stop - shape.start;

    // Map phase [0, 1] to line position [-half_length, +half_length].
    let start_pos = (start_norm * 2.0 - 1.0) * half_length;
    let end_pos = start_pos + seg_width * 2.0 * half_length;

    let left_end = -half_length;
    let right_end = half_length;

    match shape.render_mode {
        RenderMode::Arc => {
            // Draw a line segment centered at its midpoint, rotated by spin_angle.
            let draw_seg = |seg_start: f64, seg_end: f64, gl: &mut G| {
                let mid_x = (seg_start + seg_end) / 2.0;
                let seg_half_len = (seg_end - seg_start) / 2.0;
                let local = transform.trans(mid_x, y_offset).rot_rad(spin_rad);
                line::Line::new(color, thickness).draw_from_to(
                    [-seg_half_len, 0.0],
                    [seg_half_len, 0.0],
                    &Default::default(),
                    local,
                    gl,
                );
            };

            if end_pos <= right_end && start_pos >= left_end {
                draw_seg(start_pos, end_pos, gl);
            } else {
                let clamped_start = start_pos.max(left_end);
                let clamped_end = end_pos.min(right_end);
                if clamped_start < clamped_end {
                    draw_seg(clamped_start, clamped_end, gl);
                }
                if end_pos > right_end {
                    let overflow = end_pos - right_end;
                    let wrap_end = (left_end + overflow).min(right_end);
                    draw_seg(left_end, wrap_end, gl);
                }
                if start_pos < left_end {
                    let underflow = left_end - start_pos;
                    let wrap_start = (right_end - underflow).max(left_end);
                    draw_seg(wrap_start, right_end, gl);
                }
            }
        }
        RenderMode::Dot => {
            let mid_pos = (start_pos + end_pos) / 2.0;
            let fade_zone = seg_width * 2.0 * half_length;
            let wrap = compute_line_wrap(mid_pos, left_end, right_end, fade_zone);

            let r = thickness * wrap.fade;
            if r > 0.0 {
                let bound = rectangle::centered([wrap.mid, y_offset, r, r]);
                ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
            }

            if let Some(mirror_pos) = wrap.mirror_pos {
                let mirror_r = thickness * (1.0 - wrap.fade);
                if mirror_r > 0.0 {
                    let bound = rectangle::centered([mirror_pos, y_offset, mirror_r, mirror_r]);
                    ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
                }
            }
        }
        RenderMode::Saucer => {
            let mid_pos = (start_pos + end_pos) / 2.0;
            let seg_len = (end_pos - start_pos).abs();
            let fade_zone = seg_width * 2.0 * half_length;
            let wrap = compute_line_wrap(mid_pos, left_end, right_end, fade_zone);

            // Draw saucer oriented along the line (no tangent calculation needed
            // since the line direction is constant along x-axis before rotation).
            let major = seg_len / 2.0 * wrap.fade;
            if major > 0.0 {
                let bound = rectangle::centered([0.0, 0.0, major, thickness]);
                let local = transform.trans(wrap.mid, y_offset).rot_rad(spin_rad);
                ellipse::Ellipse::new(color).draw(bound, &Default::default(), local, gl);
            }

            if let Some(mirror_pos) = wrap.mirror_pos {
                let mirror_major = seg_len / 2.0 * (1.0 - wrap.fade);
                if mirror_major > 0.0 {
                    let bound = rectangle::centered([0.0, 0.0, mirror_major, thickness]);
                    let local = transform.trans(mirror_pos, y_offset).rot_rad(spin_rad);
                    ellipse::Ellipse::new(color).draw(bound, &Default::default(), local, gl);
                }
            }
        }
    }
}

impl<G: Graphics> Draw<G> for Snapshot {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        self.layers.draw(c, gl, cfg);
    }
}
