use opengl_graphics::GlGraphics;
use graphics::{Context, circle_arc, rectangle, Transformed};

use receive::{Snapshot, ArcSegment};
use config::ClientConfig;

use graphics::types::Color;

use std::f64::consts::PI;
const TWOPI: f64 = 2.0 * PI;

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
        let var_h = if hue == 1.0 {0.0} else {hue * 6.0};

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
            _ => color_from_rgb(val, var_1, var_2, alpha)
        }
    }
}

trait Draw {
    fn draw(&self, c: &Context, gl: &mut GlGraphics, cfg: &ClientConfig);
}

impl Draw for ArcSegment {
    fn draw(&self, c: &Context, gl: &mut GlGraphics, cfg: &ClientConfig) {
        let thickness = self.thickness * cfg.critical_size * cfg.thickness_scale;

        let (val, alpha) =
            if cfg.alpha_blend {(self.val, self.level)}
            else {(self.val * self.level, 1.0)};

        let color = hsv_to_rgb(self.hue, self.sat, val, alpha);

        let x = self.x * cfg.x_resolution as f64 + cfg.x_center;
        let y = self.y * cfg.y_resolution as f64 + cfg.y_center;
        let transform = c.transform.trans(x, y).rot_rad(self.rot_angle*TWOPI);

        let x_size = self.rad_x * cfg.critical_size;
        let y_size = self.rad_y * cfg.critical_size;

        let bound = rectangle::centered([0.0, 0.0, x_size, y_size]);

        circle_arc(
            color,
            thickness,
            self.start*TWOPI,
            self.stop*TWOPI,
            bound,
            transform,
            gl);
    }
}