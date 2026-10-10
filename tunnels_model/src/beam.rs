use crate::layer::{Layer, PaintMode};
use crate::render_context::RenderContext;
use crate::{look::Look, tunnel::Tunnel};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tunnels_lib::audio::AudioState;
use tunnels_lib::number::UnipolarFloat;

/// Union type for all of the kinds of beams we can have.
/// Since we don't need beam to be very extensible, we will try this approach
/// instead of having to either treat beams as trait objects or store them in
/// disparate collections.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Beam {
    Tunnel(Tunnel),
    Look(Look),
}

impl Beam {
    pub fn update_state(&mut self, delta_t: Duration, audio: &AudioState) {
        match self {
            Self::Tunnel(t) => t.update_state(delta_t, audio),
            Self::Look(l) => l.update_state(delta_t, audio),
        }
    }

    /// Append this beam's layers to `out`.
    ///
    /// A tunnel contributes one layer; a look contributes one per subchannel,
    /// and its subchannels may themselves hold looks.
    pub fn render<'f>(
        &self,
        level: UnipolarFloat,
        mode: PaintMode,
        ctx: RenderContext<'_, 'f>,
        out: &mut Vec<Layer<'f>>,
    ) {
        match self {
            Self::Tunnel(t) => out.push(t.render(level, mode, ctx)),
            Self::Look(l) => l.render(level, mode, ctx, out),
        }
    }
}
