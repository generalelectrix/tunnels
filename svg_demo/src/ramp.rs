//! The color ramp: one cycle of the hue waveform, baked into a texture.
//!
//! Sampling a ramp per pixel is what keeps color out of the mesh. The sawtooth
//! is discontinuous, and resolving that jump with per-vertex color forces the
//! geometry to be refined against it — which means every color knob, and every
//! frame of an animated one, invalidates the mesh. A texture lookup resolves it
//! at the fragment instead, so the mesh only ever has to carry phase, and a
//! color change is a small buffer rewrite.

use crate::anim::LiveWave;
use crate::params::{AnimTarget, LayerParams};
use image::{Rgba, RgbaImage};

/// Texels across one cycle of the waveform.
///
/// The jump sits between two adjacent texels, so this sets how sharp it can be:
/// with a cycle spanning a few hundred pixels on screen, a thousand texels puts
/// the transition comfortably inside one pixel.
pub const RAMP_TEXELS: u32 = 1024;

/// Convert HSV to RGB. Matches `tunnelclient::draw::hsv_to_rgb` so colors read
/// the same as they do in the real client.
fn hsv_to_rgb(hue: f64, sat: f64, val: f64, alpha: f64) -> [f64; 4] {
    let rgb = |r: f64, g: f64, b: f64| [r, g, b, alpha];
    if sat == 0.0 {
        return rgb(val, val, val);
    }
    let hue = hue.rem_euclid(1.0);
    let var_h = if hue == 1.0 { 0.0 } else { hue * 6.0 };
    let var_i = var_h.floor();
    let var_1 = val * (1.0 - sat);
    let var_2 = val * (1.0 - sat * (var_h - var_i));
    let var_3 = val * (1.0 - sat * (1.0 - (var_h - var_i)));
    match var_i as i64 {
        0 => rgb(val, var_3, var_1),
        1 => rgb(var_2, val, var_1),
        2 => rgb(var_1, val, var_3),
        3 => rgb(var_1, var_2, val),
        4 => rgb(var_3, var_1, val),
        _ => rgb(val, var_1, var_2),
    }
}

/// The rising sawtooth `Tunnel` colors with, on [0, 1) returning [-1, 1).
///
/// Ported from `waveforms::sawtooth` with smoothing off, pulse off, and a full
/// duty cycle — the settings `Tunnel::render` passes when it builds a hue.
fn sawtooth(phase: f64) -> f64 {
    let phase = phase.rem_euclid(1.0);
    if phase < 0.5 { 2.0 * phase } else { 2.0 * (phase - 1.0) }
}

/// One cycle of a layer's color waveform as a texture, to be sampled with
/// repeating wrap so the cycle count falls out of the texture coordinate.
pub fn build(layer: &LayerParams) -> RgbaImage {
    let mut img = RgbaImage::new(RAMP_TEXELS, 1);
    build_into(&mut img, layer, &[]);
    img
}

/// As `build`, reusing an existing buffer and folding in the layer's
/// color-targeted animations.
///
/// Every color animation is resolved here, once per texel, rather than per
/// vertex or per pixel. That is why an animated color costs the same as a still
/// one: however fast a waveform moves, the frame's work is a thousand
/// evaluations and one texture write.
pub fn build_into(img: &mut RgbaImage, layer: &LayerParams, waves: &[(AnimTarget, &LiveWave)]) {
    for x in 0..RAMP_TEXELS {
        let phase = f64::from(x) / f64::from(RAMP_TEXELS);

        // The base model: `Tunnel`'s own colour spread.
        let mut hue = layer.col_center + 0.5 * layer.col_width * sawtooth(phase);
        let mut sat = layer.col_sat;
        let mut level = layer.level;

        // Animations add to it, the way `col_center_adjust` does in the real
        // per-segment render.
        for (target, wave) in waves {
            let v = f64::from(wave.value_f32(phase as f32, x as usize));
            match target {
                AnimTarget::Hue => hue += 0.5 * v,
                AnimTarget::Saturation => sat = (sat + v).clamp(0.0, 1.0),
                // Only ever darkens: a light cannot exceed full.
                AnimTarget::Brightness => level *= (1.0 + v).clamp(0.0, 1.0),
                _ => {}
            }
        }

        let c = hsv_to_rgb(hue, sat, 1.0, level);
        img.put_pixel(
            x,
            0,
            Rgba([
                (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                (c[2].clamp(0.0, 1.0) * 255.0) as u8,
                (c[3].clamp(0.0, 1.0) * 255.0) as u8,
            ]),
        );
    }
}

/// The knobs a ramp is built from, so it can be rebuilt only when they move.
#[derive(PartialEq, Clone, Copy)]
pub struct RampKey([i64; 4]);

impl RampKey {
    /// Quantised to half a texel, since a change finer than that cannot alter a
    /// single entry in the table and so cannot alter a pixel on screen.
    pub fn of(layer: &LayerParams) -> Self {
        let step = f64::from(RAMP_TEXELS) * 2.0;
        Self(
            [
                layer.col_center,
                layer.col_width,
                layer.col_sat,
                layer.level,
            ]
            .map(|v| (v * step).round() as i64),
        )
    }
}
