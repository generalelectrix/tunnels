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
            let x_size = shape.extent_x * cfg.critical_size;
            let y_size = shape.extent_y * cfg.critical_size;
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
            let cx = shape.extent_x * cfg.critical_size * mid_angle.cos();
            let cy = shape.extent_y * cfg.critical_size * mid_angle.sin();
            let bound = rectangle::centered([cx, cy, thickness, thickness]);
            ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
        }
        RenderMode::Saucer => {
            let mid_angle = (shape.start + shape.stop) / 2.0 * TWOPI;
            let rx = shape.extent_x * cfg.critical_size;
            let ry = shape.extent_y * cfg.critical_size;
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

/// Cross-fade geometry for a segment whose extent crosses the right boundary
/// of the line path. Segments fully inside the line need no wrapping.
struct LineCrossfade {
    /// Center of the inside portion (for the primary shape).
    inside_mid: f64,
    /// Center of the wrapped overflow portion (for the mirror shape).
    overflow_mid: f64,
    /// Fraction of the segment inside the line [0, 1].
    inside_fraction: f64,
}

/// If the segment [start_pos, end_pos] crosses right_end, return the crossfade
/// geometry. Otherwise return None — the segment is fully inside and should be
/// drawn at full size at its midpoint.
fn compute_line_crossfade(
    start_pos: f64,
    end_pos: f64,
    left_end: f64,
    right_end: f64,
) -> Option<LineCrossfade> {
    if end_pos <= right_end {
        return None;
    }
    let seg_len = end_pos - start_pos;
    if seg_len <= 0.0 {
        return None;
    }
    let inside_fraction = ((right_end - start_pos) / seg_len).clamp(0.0, 1.0);
    let overflow = end_pos - right_end;
    Some(LineCrossfade {
        inside_mid: (start_pos + right_end) / 2.0,
        overflow_mid: left_end + overflow / 2.0,
        inside_fraction,
    })
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
    let half_length = shape.extent_x * cfg.critical_size;
    let y_offset = shape.extent_y * cfg.critical_size;

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
            let draw_seg = |mid_x: f64, seg_half_len: f64, gl: &mut G| {
                let local = transform.trans(mid_x, y_offset).rot_rad(spin_rad);
                line::Line::new(color, thickness).draw_from_to(
                    [-seg_half_len, 0.0],
                    [seg_half_len, 0.0],
                    &Default::default(),
                    local,
                    gl,
                );
            };

            let seg_half_len = (end_pos - start_pos) / 2.0;
            if let Some(cf) = compute_line_crossfade(start_pos, end_pos, left_end, right_end) {
                // Inside portion.
                let inside_half = seg_half_len * cf.inside_fraction;
                if inside_half > 0.0 {
                    draw_seg(cf.inside_mid, inside_half, gl);
                }
                // Overflow portion wrapped to the left end.
                let overflow_half = seg_half_len * (1.0 - cf.inside_fraction);
                if overflow_half > 0.0 {
                    draw_seg(cf.overflow_mid, overflow_half, gl);
                }
            } else {
                let mid_pos = (start_pos + end_pos) / 2.0;
                draw_seg(mid_pos, seg_half_len, gl);
            }
        }
        RenderMode::Dot => {
            if let Some(cf) = compute_line_crossfade(start_pos, end_pos, left_end, right_end) {
                // Segment crosses the right boundary — crossfade.
                let r = thickness * cf.inside_fraction;
                if r > 0.0 {
                    let bound = rectangle::centered([cf.inside_mid, y_offset, r, r]);
                    ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
                }
                let mirror_r = thickness * (1.0 - cf.inside_fraction);
                if mirror_r > 0.0 {
                    let bound =
                        rectangle::centered([cf.overflow_mid, y_offset, mirror_r, mirror_r]);
                    ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
                }
            } else {
                // Fully inside — draw at full size.
                let mid_pos = (start_pos + end_pos) / 2.0;
                let bound = rectangle::centered([mid_pos, y_offset, thickness, thickness]);
                ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
            }
        }
        RenderMode::Saucer => {
            let seg_len = end_pos - start_pos;
            if let Some(cf) = compute_line_crossfade(start_pos, end_pos, left_end, right_end) {
                // Segment crosses the right boundary — crossfade.
                // Draw saucer oriented along the line.
                let major = seg_len / 2.0 * cf.inside_fraction;
                if major > 0.0 {
                    let bound = rectangle::centered([0.0, 0.0, major, thickness]);
                    let local = transform.trans(cf.inside_mid, y_offset).rot_rad(spin_rad);
                    ellipse::Ellipse::new(color).draw(bound, &Default::default(), local, gl);
                }
                let mirror_major = seg_len / 2.0 * (1.0 - cf.inside_fraction);
                if mirror_major > 0.0 {
                    let bound = rectangle::centered([0.0, 0.0, mirror_major, thickness]);
                    let local = transform.trans(cf.overflow_mid, y_offset).rot_rad(spin_rad);
                    ellipse::Ellipse::new(color).draw(bound, &Default::default(), local, gl);
                }
            } else {
                // Fully inside — draw at full size.
                let mid_pos = (start_pos + end_pos) / 2.0;
                let major = seg_len / 2.0;
                if major > 0.0 {
                    let bound = rectangle::centered([0.0, 0.0, major, thickness]);
                    let local = transform.trans(mid_pos, y_offset).rot_rad(spin_rad);
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
