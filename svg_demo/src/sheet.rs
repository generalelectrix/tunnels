//! Headless PNG output: contact sheets of the shape library and stacking demos.

use crate::anim::{LiveWave, WaveformKind};
use crate::draw::{
    AxisWave, GeometryWave, Shaded, color_offsets_into, draw_flat, draw_layer,
    draw_textured, flat_color, is_uniform, layer_transform, phase_uvs, phase_uvs_into,
    warp_verts_into,
};
use crate::params::{AnimTarget, TargetedWave, WaveParams};
use tunnels_lib::number::UnipolarFloat;
use crate::mesh::{self, Level, refine};
use crate::params::PhaseField;
use crate::ramp;
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
    render_cell_at(shape, layer, size, SUPERSAMPLE, mesh::DEFAULT_TARGET_PX)
}

/// As `render_cell`, but with the supersampling factor named.
///
/// Supersampling hides mesh artifacts, so a factor of one is what to use when
/// the question is how the mesh itself looks.
fn render_cell_at(
    shape: &ShapeMesh,
    layer: &LayerParams,
    size: u32,
    ss: u32,
    target_px: f64,
) -> RgbaImage {
    let hi = size * ss;
    let mut buf = RenderBuffer::new(hi, hi);
    buf.clear_color([0.0, 0.0, 0.0, 1.0]);
    let base: Matrix2d = identity().trans(f64::from(hi) / 2.0, f64::from(hi) / 2.0);
    let outline = layer
        .draw_mode
        .draws_outline()
        .then(|| shape.stroke(layer.stroke_width as f32));
    let m = layer_transform(base, layer, 0.0, f64::from(hi));
    draw_one(shape, outline.as_deref(), layer, m, f64::from(hi), target_px, &mut buf);
    downsample(&buf.into_image(), ss)
}

/// A grid showing what each animation target does to a shape.
///
/// Rows are targets, columns are waveforms. Everything is still — speed is
/// zero — because a contact sheet cannot show motion, and the shape a waveform
/// imposes is the thing worth judging.
pub fn anim_sheet(shapes: &[ShapeMesh], idx: usize, out: &Path, cell: u32) -> Result<()> {
    // The last row animates nothing: it sweeps the base spin knob, which is the
    // one geometry control with no counterpart in `Tunnel` and so the only one
    // that had to be added rather than reused.
    let rows: [(AnimTarget, ColorPhase, &str); 7] = [
        (AnimTarget::Radial, ColorPhase::Angle, "radial along angle"),
        (AnimTarget::Radial, ColorPhase::Radius, "radial along radius"),
        (AnimTarget::Spin, ColorPhase::Radius, "spin along radius"),
        (AnimTarget::AspectRatio, ColorPhase::Angle, "aspect along angle"),
        (AnimTarget::Hue, ColorPhase::Angle, "hue"),
        (AnimTarget::Hue, ColorPhase::Angle, "base spin knob, no animation"),
        (
            AnimTarget::Hue,
            ColorPhase::Angle,
            "two colour axes: hue on angle, brightness on radius",
        ),
    ];
    let cols: [(WaveformKind, u16, f64); 5] = [
        (WaveformKind::Sine, 3, 0.35),
        (WaveformKind::Sine, 8, 0.2),
        (WaveformKind::Square, 5, 0.3),
        (WaveformKind::Sawtooth, 4, 0.3),
        (WaveformKind::Noise, 6, 0.4),
    ];

    let gap = 6;
    let pitch = cell + gap;
    let mut sheet = RgbaImage::from_pixel(
        cols.len() as u32 * pitch + gap,
        rows.len() as u32 * pitch + gap,
        image::Rgba([24, 24, 28, 255]),
    );

    let base_spin_row = rows.len() - 2;
    let two_axis_row = rows.len() - 1;
    for (r, (target, phase, name)) in rows.iter().enumerate() {
        for (c, (waveform, n_periods, size)) in cols.iter().enumerate() {
            let mut layer = LayerParams {
                enabled: true,
                shape: idx,
                scale_x: 0.72,
                scale_y: 0.72,
                color_phase: ColorPhase::Angle,
                col_center: 0.5,
                col_width: 0.5,
                col_spread: 0.15,
                col_sat: 0.85,
                ..Default::default()
            };
            if r == base_spin_row {
                // Sweep the knob across the row instead of varying a waveform,
                // and hold the colour flat. A uniform layer takes a different
                // path through the renderer, and geometry has to survive it —
                // it did not, which is how the spin knob came to do nothing.
                layer.spin = -0.6 + 0.3 * c as f64;
                layer.col_width = 0.0;
            } else {
                layer.waves[0] = TargetedWave {
                    enabled: true,
                    target: *target,
                    phase: *phase,
                    wave: WaveParams {
                        waveform: *waveform,
                        n_periods: *n_periods,
                        size: *size,
                        ..Default::default()
                    },
                };
                if r == two_axis_row {
                    // Hue indexes the ramp along the angle; brightness rides the
                    // vertices along the radius. Two independent axes at once,
                    // which one ramp coordinate could not carry.
                    layer.waves[1] = TargetedWave {
                        enabled: true,
                        target: AnimTarget::Brightness,
                        phase: ColorPhase::Radius,
                        wave: WaveParams {
                            waveform: WaveformKind::Sine,
                            n_periods: 3 + c as u16,
                            size: 0.9,
                            ..Default::default()
                        },
                    };
                }
            }
            let img = render_animated(&shapes[idx], &layer, cell);
            paste(&mut sheet, &img, gap + c as u32 * pitch, gap + r as u32 * pitch);
        }
        println!("row {r}: {name}");
    }
    sheet.save(out)?;
    Ok(())
}

/// Render one shape with its animation slots applied.
fn render_animated(shape: &ShapeMesh, layer: &LayerParams, size: u32) -> RgbaImage {
    let ss = SUPERSAMPLE;
    let hi = size * ss;
    let mut buf = RenderBuffer::new(hi, hi);
    buf.clear_color([0.0, 0.0, 0.0, 1.0]);
    let base: Matrix2d = identity().trans(f64::from(hi) / 2.0, f64::from(hi) / 2.0);
    let m = layer_transform(base, layer, 0.0, f64::from(hi));
    let audio = UnipolarFloat::ZERO;

    let live: Vec<LiveWave> = layer.waves.iter().map(|w| LiveWave::new(&w.wave)).collect();
    let active = || {
        layer
            .waves
            .iter()
            .enumerate()
            .filter(|(_, w)| w.enabled && w.wave.size > 0.0)
    };

    // Mirror the render loop's split: only the layer's own colour axis indexes
    // the ramp; anything else reaches the fragment through the vertices.
    let color_waves: Vec<_> = active()
        .filter(|(_, w)| w.target.is_color() && w.phase == layer.color_phase)
        .map(|(i, w)| (w.target, &live[i]))
        .collect();
    let off_axis = |target| -> Vec<AxisWave> {
        active()
            .filter(|(_, w)| w.target == target && w.phase != layer.color_phase)
            .map(|(i, w)| AxisWave {
                target: w.target,
                phase: w.phase,
                wave: &live[i],
            })
            .collect()
    };
    let hue_axes = off_axis(AnimTarget::Hue);
    let bright_axes = off_axis(AnimTarget::Brightness);
    let mut ramp_img = RgbaImage::new(ramp::RAMP_TEXELS, 1);
    ramp::build_into(&mut ramp_img, layer, &color_waves, audio);
    let texture = RenderBuffer::from_image(ramp_img);

    let warps: Vec<GeometryWave> = active()
        .filter(|(_, w)| !w.target.is_color())
        .map(|(i, w)| GeometryWave {
            target: w.target,
            phase: w.phase,
            wave: &live[i],
        })
        .collect();

    let scale = layer.scale_x.abs().max(layer.scale_y.abs());
    let target = Level::for_scale(scale, f64::from(hi), mesh::DEFAULT_TARGET_PX).target_edge();
    let refined = refine(&shape.fill, target);
    let field = PhaseField::of(layer);

    let mut positions = Vec::new();
    if warps.is_empty() && layer.spin == 0.0 {
        positions.extend_from_slice(&refined.verts);
    } else {
        warp_verts_into(&mut positions, &refined, layer.spin as f32, &warps, audio);
    }
    // Mirror the render loop's own decision, so this sheet exercises the same
    // paths the window does rather than a convenient subset of them.
    if is_uniform(layer) || layer.mask {
        draw_flat(&refined, &positions, flat_color(layer), m, &mut buf);
    } else {
        let mut hue_offsets = Vec::new();
        let mut tints = Vec::new();
        color_offsets_into(
            &mut hue_offsets,
            &mut tints,
            &refined,
            &hue_axes,
            &bright_axes,
            audio,
        );
        let mut uvs = Vec::new();
        phase_uvs_into(
            &mut uvs,
            &refined,
            field,
            (!hue_axes.is_empty()).then_some(hue_offsets.as_slice()),
        );
        draw_textured(
            &refined,
            Shaded {
                positions: &positions,
                uvs: &uvs,
                tints: (!bright_axes.is_empty()).then_some(tints.as_slice()),
            },
            field.wrap_period(),
            &texture,
            m,
            &mut buf,
        );
    }
    downsample(&buf.into_image(), ss)
}

/// Draw a layer, refining it first if its color varies across the shape.
fn draw_one(
    shape: &ShapeMesh,
    outline: Option<&[[f32; 2]]>,
    layer: &LayerParams,
    m: graphics::math::Matrix2d,
    critical: f64,
    target_px: f64,
    buf: &mut RenderBuffer,
) {
    if (is_uniform(layer) || layer.mask) && layer.spin == 0.0 {
        draw_layer(shape, outline, layer, m, buf);
        return;
    }
    let scale = layer.scale_x.abs().max(layer.scale_y.abs());
    let target = Level::for_scale(scale, critical, target_px).target_edge();
    let field = PhaseField::of(layer);
    let texture = RenderBuffer::from_image(ramp::build(layer));
    if layer.draw_mode.draws_fill() {
        let mesh = refine(&shape.fill, target);
        draw_textured(
            &mesh,
            Shaded {
                positions: &mesh.verts,
                uvs: &phase_uvs(&mesh, field),
                tints: None,
            },
            field.wrap_period(),
            &texture,
            m,
            buf,
        );
    }
    if let Some(outline) = outline {
        let mesh = refine(outline, target);
        draw_textured(
            &mesh,
            Shaded {
                positions: &mesh.verts,
                uvs: &phase_uvs(&mesh, field),
                tints: None,
            },
            field.wrap_period(),
            &texture,
            m,
            buf,
        );
    }
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

/// One shape, large, for inspecting mesh and gradient quality up close.
pub fn zoom(
    shape: &ShapeMesh,
    out: &Path,
    size: u32,
    phase: ColorPhase,
    supersample: u32,
    target_px: f64,
) -> Result<()> {
    let layer = LayerParams {
        enabled: true,
        scale_x: 0.92,
        scale_y: 0.92,
        color_phase: phase,
        col_center: 0.45,
        col_width: 0.8,
        col_spread: 0.2,
        col_sat: 0.9,
        ..Default::default()
    };
    render_cell_at(shape, &layer, size, supersample, target_px).save(out)?;
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
        let m = layer_transform(base, layer, 0.0, f64::from(hi));
        draw_one(&shapes[idx], None, layer, m, f64::from(hi), mesh::DEFAULT_TARGET_PX, &mut buf);
    }
    downsample(&buf.into_image(), SUPERSAMPLE)
}
