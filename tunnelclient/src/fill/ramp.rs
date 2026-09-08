//! The colour ramp: one cycle of a figure's colour, baked into a texture.
//!
//! Sampling a ramp per fragment is what keeps colour out of the mesh. The
//! sawtooth a beam colours with is discontinuous, and resolving that jump with
//! per-vertex colour forces the geometry to be refined against it — which means
//! every colour knob, and every frame of an animated one, invalidates the mesh.
//! A texture lookup resolves it at the fragment instead, so the mesh only ever
//! has to carry phase, and a colour change is a small buffer rewrite.

use crate::draw::hsv_to_rgb;
use image::{Rgba, RgbaImage};
use tunnels_lib::number::Phase;
use tunnels_model::animation::{PreparedAnimation, TargetedAnimation};
use tunnels_model::animation_target::AnimationTarget;
use tunnels_model::layer::{ColorAdjust, ColorField};

/// Texels across one cycle of the waveform.
///
/// The jump sits between two adjacent texels, so this sets how sharp it can be:
/// with a cycle spanning a few hundred pixels on screen, a thousand texels puts
/// the transition comfortably inside one pixel.
pub const RAMP_TEXELS: u32 = 1024;

/// A blank ramp of the right size, to be filled before it is drawn with.
pub fn blank() -> RgbaImage {
    RgbaImage::new(RAMP_TEXELS, 1)
}

/// Write one cycle of a layer's colour into `img`.
///
/// Every colour animation is resolved here, once per texel, rather than per
/// vertex or per pixel. That is why an animated colour costs the same as a
/// still one: however fast a waveform moves, the frame's work is a thousand
/// evaluations and one texture write.
pub fn build_into(
    img: &mut RgbaImage,
    color: &ColorField,
    anims: &[TargetedAnimation<PreparedAnimation>],
) {
    for x in 0..RAMP_TEXELS {
        let phase = Phase::new(f64::from(x) / f64::from(RAMP_TEXELS));

        let mut adjust = ColorAdjust::default();
        for anim in anims {
            let value = anim.animation.value(phase, x as usize);
            // The same adjustments a beam makes to its own colour, against
            // the ramp's coordinate instead of a segment index.
            match anim.target {
                AnimationTarget::Color => adjust.center += value * 0.5,
                AnimationTarget::ColorSpread => adjust.width += value,
                AnimationTarget::ColorSaturation => adjust.sat += value,
                _ => {}
            }
        }

        let sampled = color.sample(phase, adjust);
        let c = hsv_to_rgb(sampled.hue, sampled.sat, sampled.val, sampled.level);
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
