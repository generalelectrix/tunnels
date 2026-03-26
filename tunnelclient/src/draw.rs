use std::sync::Arc;

use crate::config::ClientConfig;
use graphics::types::Color;
use graphics::Context;
use graphics::{ellipse, rectangle, CircleArc, Graphics, Transformed};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use tunnels_lib::{RenderMode, Shape, Snapshot};

const TWOPI: f64 = 2.0 * PI;

/// The axis along which to perform a transformation.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum TransformDirection {
    Vertical,
    Horizontal,
}

/// Action and direction of a geometric transformation to perform.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Transform {
    /// Flip the image in the specified direction.
    Flip(TransformDirection),
    // /// Mirror the image in the specified direction.
    //Mirror(TransformDirection),
}

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

        match self.render_mode {
            RenderMode::Arc => {
                let thickness = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;
                let x_size = self.rad_x * cfg.critical_size;
                let y_size = self.rad_y * cfg.critical_size;
                let bound = rectangle::centered([0.0, 0.0, x_size, y_size]);
                let start = self.start * TWOPI;
                let stop = self.stop * TWOPI;
                CircleArc::new(color, thickness, start, stop).draw(
                    bound,
                    &Default::default(),
                    transform,
                    gl,
                );
            }
            RenderMode::Dot => {
                let dot_radius = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;
                let mid_angle = (self.start + self.stop) / 2.0 * TWOPI;
                let cx = self.rad_x * cfg.critical_size * mid_angle.cos();
                let cy = self.rad_y * cfg.critical_size * mid_angle.sin();
                let bound = rectangle::centered([cx, cy, dot_radius, dot_radius]);
                ellipse::Ellipse::new(color).draw(bound, &Default::default(), transform, gl);
            }
            RenderMode::Saucer => {
                // Centroid: same as Dot mode
                let mid_angle = (self.start + self.stop) / 2.0 * TWOPI;
                let rx = self.rad_x * cfg.critical_size;
                let ry = self.rad_y * cfg.critical_size;
                let cx = rx * mid_angle.cos();
                let cy = ry * mid_angle.sin();

                // Major axis: chord length between start and end points on ellipse
                let start_rad = self.start * TWOPI;
                let stop_rad = self.stop * TWOPI;
                let p1x = rx * start_rad.cos();
                let p1y = ry * start_rad.sin();
                let p2x = rx * stop_rad.cos();
                let p2y = ry * stop_rad.sin();
                let chord_len = ((p2x - p1x).powi(2) + (p2y - p1y).powi(2)).sqrt() / 2.0;

                // Minor axis: thickness (same scaling as Dot mode radius)
                let minor_radius = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;

                // Orientation: tangent to ellipse path at midpoint
                // For parametric ellipse (rx*cos(t), ry*sin(t)), tangent at t is (-rx*sin(t), ry*cos(t))
                let tangent_angle = (ry * mid_angle.cos()).atan2(-rx * mid_angle.sin());

                // Center bound at origin, then transform: rotate by tangent, translate to centroid.
                // This ensures rotation happens around the saucer's center, not around (0,0).
                let bound = rectangle::centered([0.0, 0.0, chord_len, minor_radius]);
                let local_transform = transform
                    .trans(cx, cy)
                    .rot_rad(tangent_angle + self.spin_angle * TWOPI);
                ellipse::Ellipse::new(color).draw(bound, &Default::default(), local_transform, gl);
            }
        }
    }
}

impl<G: Graphics> Draw<G> for Snapshot {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        self.layers.draw(c, gl, cfg);
    }
}
