use graphics::{Context, CircleArc, rectangle, Transformed, Graphics, DrawState};

use receive::{Snapshot, ArcSegment};
use config::ClientConfig;

use graphics::types::Color;
use graphics::types::{Matrix2d, Scalar, Resolution, Radius, Rectangle};
use graphics::radians::Radians;
use graphics::triangulation::stream_quad_tri_list;

use constants::TWOPI;

pub trait Draw<G: Graphics> {
    /// Given a context and gl instance, draw this entity to the screen.
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig);
}

impl<T, G> Draw<G> for Vec<T> where
        G: Graphics,
        T: Draw<G> {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        for e in self {
            e.draw(c, gl, cfg);
        }
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

/// Draws circle arc using triangulation.
pub fn draw_circle_arc_improved<R: Into<Rectangle>, G>(
    ca: &CircleArc,
    rectangle: R,
    draw_state: &DrawState,
    transform: Matrix2d,
    g: &mut G
)
    where G: Graphics
{
    let rectangle = rectangle.into();
    g.tri_list(
        &draw_state,
        &ca.color,
        |f|
    improved_with_arc_tri_list(
        ca.start,
        ca.end,
        ca.resolution,
        transform,
        rectangle,
        ca.radius,
        |vertices| f(vertices)
    ));
}

/// Streams an arc between the two radian boundaries.
#[inline(always)]
fn improved_with_arc_tri_list<F>(
    start_radians: Scalar,
    end_radians: Scalar,
    resolution: Resolution,
    m: Matrix2d,
    rect: Rectangle,
    border_radius: Radius,
    f: F
)
    where
        F: FnMut(&[f32])
{

    let (x, y, w, h) = (rect[0], rect[1], rect[2], rect[3]);
    let (cw, ch) = (0.5 * w, 0.5 * h);
    let (cw1, ch1) = (cw + border_radius, ch + border_radius);
    let (cw2, ch2) = (cw - border_radius, ch - border_radius);
    let (cx, cy) = (x + cw, y + ch);
    let default_seg_size = <Scalar as Radians>::_360() / resolution as Scalar;
    let mut i = 0;
    let (start, end) = if start_radians < end_radians {
        (start_radians, end_radians)
    } else {
        (end_radians, start_radians)
    };
    // Use a similar number of quads as default method, but ensure the angular
    // delta allows for exactly spanning the included angle.
    let delta = end - start;
    let n_quads = (delta / default_seg_size).ceil() as u64;
    let seg_size = delta / n_quads as f64;
    stream_quad_tri_list(m, || {
        if i > n_quads { return None; }

        let angle = start + (i as f64 * seg_size);

        let cos = angle.cos();
        let sin = angle.sin();
        i += 1;
        Some(([cx + cos * cw1, cy + sin * ch1],
            [cx + cos * cw2, cy + sin * ch2]))
    }, f);
}

impl<G: Graphics> Draw<G> for ArcSegment {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        let thickness = self.thickness * cfg.critical_size * cfg.thickness_scale / 2.0;

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

        let start = self.start * TWOPI;
        let stop = self.stop * TWOPI;

        let ca = CircleArc::new(color, thickness, start, stop);
        //ca.draw(bound, &Default::default(), transform, gl);
        draw_circle_arc_improved(&ca, bound, &Default::default(), transform, gl);
    }
}

impl<G: Graphics> Draw<G> for Snapshot {
    fn draw(&self, c: &Context, gl: &mut G, cfg: &ClientConfig) {
        self.layers.draw(c, gl, cfg);
    }
}