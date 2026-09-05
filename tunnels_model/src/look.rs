use crate::mixer::Channel;
use crate::render_context::RenderContext;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tunnels_lib::Layer;
use tunnels_lib::number::UnipolarFloat;

/// A look is a beam that is essentially the contents of an entire mixer.
/// All channel settings are preserved.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Look {
    pub channels: Vec<Channel>,
}

impl Look {
    pub fn from_channels(channels: Vec<Channel>) -> Self {
        Self { channels }
    }

    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        for channel in &mut self.channels {
            channel.update_state(delta_t, audio_envelope);
        }
    }

    /// Draw all the Beams in this Look.
    ///
    /// Each subchannel contributes its own layers, so a look never merges shapes
    /// that are drawn differently into one layer.
    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        ctx: RenderContext,
        out: &mut Vec<Layer>,
    ) {
        for channel in &self.channels {
            channel.render(level, mask, ctx, out);
        }
    }
}
