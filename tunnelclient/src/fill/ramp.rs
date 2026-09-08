//! The colour ramp: a figure's colour, baked into a texture.
//!
//! Sampling a ramp per fragment is what keeps colour out of the mesh. The
//! sawtooth a beam colours with is discontinuous, and resolving that jump with
//! per-vertex colour forces the geometry to be refined against it — which means
//! every colour knob, and every frame of an animated one, invalidates the mesh.
//! A texture lookup resolves it at the fragment instead, so the mesh only ever
//! has to carry phase, and a colour change is a small buffer rewrite.
//!
//! Per-vertex colour cannot be brought back for the animated case either, and
//! the reason is worth keeping: the only per-vertex channel this pipeline
//! offers is a colour *multiplied* against the sampled texel. A multiply can
//! darken and it can fade, but it cannot desaturate and it cannot move a hue,
//! so of the three colour targets it carries none. The prototype's vertex
//! tints held brightness for exactly that reason.

use crate::draw::hsv_to_rgb;
use image::{Rgba, RgbaImage};
use tunnels_lib::number::Phase;
use tunnels_model::animation::{PreparedAnimation, TargetedAnimation};
use tunnels_model::animation_target::AnimationTarget;
use tunnels_model::layer::{ColorAdjust, ColorField};

/// Texels across one colour cycle, when the ramp holds a cycle.
///
/// The jump sits between two adjacent texels, so this sets how sharp it can be:
/// with a cycle spanning a few hundred pixels on screen, a thousand texels puts
/// the transition comfortably inside one pixel.
const CYCLE_TEXELS: u32 = 1024;

/// Texels across the whole figure, when the ramp holds a figure.
///
/// A figure-wide ramp packs every cycle into one table, so at the highest
/// spread it holds sixteen of them and each gets a sixteenth of the texels.
/// Four times the length is what brings a cycle's share back under a screen
/// pixel at the size a figure is actually projected: measured on a 2048-pixel
/// render at maximum spread, a figure-wide ramp of this length sits within five
/// parts in 255 of a cycle-wide one, and a thousand-texel figure ramp does not
/// — it bands the smooth stretches into steps about three pixels wide.
///
/// What does *not* degrade is the sawtooth's edge. Sampling puts the jump
/// between two texels whatever the table's length, so the jump stays one pixel
/// hard; what a coarser table moves is the jump's *position*, which quantizes
/// onto the texel grid. A reader who sees a table stretched over sixteen cycles
/// and assumes the sharp edge was traded away has it backwards.
///
/// A table sized from the figure's extent on screen rather than fixed would be
/// better still — a figure drawn small needs far fewer texels than one filling
/// the projector — but it makes the ramp's length depend on placement, and this
/// is the length that works everywhere the figure can be put.
const FIGURE_TEXELS: u32 = 4096;

/// What one ramp's width covers.
///
/// A figure's colour and its colour animations are indexed by the same
/// coordinate but at different rates: the colour repeats `cycles` times across
/// the figure, and an animation runs its own number of periods across the whole
/// of it — the way a beam's animations run across the whole beam rather than
/// once per colour cycle. One table can hold one or the other.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RampSpan {
    /// One colour cycle.
    ///
    /// The figure coordinate is multiplied by the cycle count before it indexes
    /// the ramp, so however many cycles run across a figure the table holds
    /// one. An animation resolved against this table runs a period per cycle,
    /// which is only right when it does not vary in space at all.
    Cycle,
    /// The whole figure.
    ///
    /// The figure coordinate indexes the ramp directly and the cycle count
    /// moves onto the colour's own sample, so an animation's period is the
    /// figure's and not the colour's. This costs four times the texels, and is
    /// taken only where it is needed.
    Figure,
}

impl RampSpan {
    /// The span a layer's colour animations require.
    ///
    /// Two properties make this the place to decide it. The expensive span is
    /// taken by exactly the case a cycle-wide table gets wrong — an animation
    /// whose value depends on where on the figure it is asked. And every layer
    /// without one stays on the cycle-wide table, bit for bit as before, so a
    /// render that moves is evidence of a fault rather than of this choice.
    pub fn of(anims: &[TargetedAnimation<PreparedAnimation>]) -> Self {
        if anims.iter().any(|a| a.animation.varies_in_space()) {
            Self::Figure
        } else {
            Self::Cycle
        }
    }

    /// Texels across this span.
    pub fn texels(self) -> u32 {
        match self {
            Self::Cycle => CYCLE_TEXELS,
            Self::Figure => FIGURE_TEXELS,
        }
    }
}

/// A blank ramp of the right size for a span, to be filled before it is drawn
/// with.
pub fn blank(span: RampSpan) -> RgbaImage {
    RgbaImage::new(span.texels(), 1)
}

/// Write a layer's colour into `img`, across the span `img` is sized for.
///
/// Every colour animation is resolved here, once per texel, rather than per
/// vertex or per pixel. That is why an animated colour costs the same as a
/// still one: however fast a waveform moves, the frame's work is a few thousand
/// evaluations and one texture write.
pub fn build_into(
    img: &mut RgbaImage,
    span: RampSpan,
    color: &ColorField,
    anims: &[TargetedAnimation<PreparedAnimation>],
) {
    let texels = img.width();
    for x in 0..texels {
        // Across a cycle or across the figure, according to what this table
        // holds. Either way it is the coordinate an animation is asked at, so
        // an animation's period is the span's and never the colour's.
        let phase = Phase::new(f64::from(x) / f64::from(texels));

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

        // A figure-wide table walks the colour through every one of its cycles,
        // because the coordinate indexing it covers the whole figure.
        let sampled = match span {
            RampSpan::Cycle => color.sample(phase, adjust),
            RampSpan::Figure => color.sample(Phase::new(phase.val() * color.cycles), adjust),
        };
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
