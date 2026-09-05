//! Headless PNG output: contact sheets of the shape library and stacking demos.

use crate::draw::{draw_layer, layer_transform};
use crate::params::{ColorPhase, DrawMode, LayerParams};
use crate::shapes::ShapeMesh;
use crate::software::RenderBuffer;
use anyhow::Result;
use graphics::math::{Matrix2d, identity};
use graphics::{Graphics, Transformed};
use image::RgbaImage;
use std::path::Path;

/// Rendered at this multiple of the output size, then box-filtered down. The
/// real client gets its edge quality from 4x MSAA; this approximates it.
const SUPERSAMPLE: u32 = 3;

/// Draw one shape into a square image of the given size.
fn render_cell(shape: &ShapeMesh, layer: &LayerParams, size: u32) -> RgbaImage {
    let hi = size * SUPERSAMPLE;
    let mut buf = RenderBuffer::new(hi, hi);
    buf.clear_color([0.0, 0.0, 0.0, 1.0]);
    let base: Matrix2d = identity().trans(f64::from(hi) / 2.0, f64::from(hi) / 2.0);
    let outline = layer
        .draw_mode
        .draws_outline()
        .then(|| shape.stroke(layer.stroke_width as f32));
    draw_layer(
        shape,
        outline.as_deref(),
        layer,
        layer_transform(base, layer, 0.0, f64::from(hi)),
        &mut buf,
    );
    downsample(&buf.into_image(), SUPERSAMPLE)
}

fn downsample(src: &RgbaImage, factor: u32) -> RgbaImage {
    let (w, h) = (src.width() / factor, src.height() / factor);
    let mut out = RgbaImage::new(w, h);
    let n = f32::from(factor as u16 * factor as u16);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0f32; 4];
            for dy in 0..factor {
                for dx in 0..factor {
                    let p = src.get_pixel(x * factor + dx, y * factor + dy);
                    for c in 0..4 {
                        acc[c] += f32::from(p[c]);
                    }
                }
            }
            out.put_pixel(
                x,
                y,
                image::Rgba([
                    (acc[0] / n) as u8,
                    (acc[1] / n) as u8,
                    (acc[2] / n) as u8,
                    (acc[3] / n) as u8,
                ]),
            );
        }
    }
    out
}

fn paste(sheet: &mut RgbaImage, cell: &RgbaImage, ox: u32, oy: u32) {
    for y in 0..cell.height() {
        for x in 0..cell.width() {
            if ox + x < sheet.width() && oy + y < sheet.height() {
                sheet.put_pixel(ox + x, oy + y, *cell.get_pixel(x, y));
            }
        }
    }
}

/// Every shape in the library, on one grid, so the set can be judged and culled.
pub fn contact_sheet(shapes: &[ShapeMesh], out: &Path, cell: u32, cols: u32) -> Result<()> {
    let rows = shapes.len().div_ceil(cols as usize) as u32;
    let gap = 4;
    let pitch = cell + gap;
    let mut sheet = RgbaImage::from_pixel(
        cols * pitch + gap,
        rows * pitch + gap,
        image::Rgba([24, 24, 28, 255]),
    );

    let layer = LayerParams {
        enabled: true,
        scale_x: 0.86,
        scale_y: 0.86,
        col_sat: 0.0,
        ..Default::default()
    };

    for (i, shape) in shapes.iter().enumerate() {
        let img = render_cell(shape, &layer, cell);
        let (cx, cy) = (i as u32 % cols, i as u32 / cols);
        paste(&mut sheet, &img, gap + cx * pitch, gap + cy * pitch);
    }
    sheet.save(out)?;
    Ok(())
}

/// A single shape at a larger size, exercising each color mode and a stack of
/// three layers with a mask — the gobo question in one image.
pub fn feature_sheet(shapes: &[ShapeMesh], picks: &[usize], out: &Path, cell: u32) -> Result<()> {
    let modes = ColorPhase::ALL;
    let gap = 6;
    let pitch = cell + gap;
    let cols = modes.len() as u32 + 2;
    let mut sheet = RgbaImage::from_pixel(
        cols * pitch + gap,
        picks.len() as u32 * pitch + gap,
        image::Rgba([24, 24, 28, 255]),
    );

    for (row, &idx) in picks.iter().enumerate() {
        let shape = &shapes[idx];
        let oy = gap + row as u32 * pitch;

        for (col, mode) in modes.iter().enumerate() {
            let layer = LayerParams {
                enabled: true,
                scale_x: 0.86,
                scale_y: 0.86,
                color_phase: *mode,
                col_center: 0.5,
                col_width: 0.7,
                col_spread: 0.2,
                col_sat: 0.85,
                ..Default::default()
            };
            let img = render_cell(shape, &layer, cell);
            paste(&mut sheet, &img, gap + col as u32 * pitch, oy);
        }

        // Outline mode.
        let outlined = LayerParams {
            enabled: true,
            scale_x: 0.86,
            scale_y: 0.86,
            draw_mode: DrawMode::Outline,
            stroke_width: 0.035,
            color_phase: ColorPhase::Angle,
            col_center: 0.08,
            col_width: 0.5,
            col_spread: 0.13,
            col_sat: 0.9,
            ..Default::default()
        };
        let img = render_cell(shape, &outlined, cell);
        paste(&mut sheet, &img, gap + modes.len() as u32 * pitch, oy);

        // Stacked: a lit shape, a rotated copy of the same shape as a mask, and
        // a second mask at another angle. The surviving light is the
        // intersection of the apertures — gobo stacking.
        let img = render_stack(shapes, idx, cell);
        paste(&mut sheet, &img, gap + (modes.len() as u32 + 1) * pitch, oy);
    }
    sheet.save(out)?;
    Ok(())
}

fn render_stack(shapes: &[ShapeMesh], idx: usize, size: u32) -> RgbaImage {
    let hi = size * SUPERSAMPLE;
    let mut buf = RenderBuffer::new(hi, hi);
    buf.clear_color([0.0, 0.0, 0.0, 1.0]);
    let base: Matrix2d = identity().trans(f64::from(hi) / 2.0, f64::from(hi) / 2.0);

    let lit = LayerParams {
        enabled: true,
        scale_x: 0.95,
        scale_y: 0.95,
        color_phase: ColorPhase::Radius,
        col_center: 0.12,
        col_width: 0.6,
        col_spread: 0.3,
        col_sat: 0.9,
        ..Default::default()
    };
    let mask_a = LayerParams {
        enabled: true,
        scale_x: 0.72,
        scale_y: 0.72,
        rotation: 0.13,
        mask: true,
        ..Default::default()
    };
    let mask_b = LayerParams {
        enabled: true,
        scale_x: 0.45,
        scale_y: 0.45,
        rotation: 0.31,
        mask: true,
        ..Default::default()
    };
    for layer in [&lit, &mask_a, &mask_b] {
        draw_layer(
            &shapes[idx],
            None,
            layer,
            layer_transform(base, layer, 0.0, f64::from(hi)),
            &mut buf,
        );
    }
    downsample(&buf.into_image(), SUPERSAMPLE)
}
