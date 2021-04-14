use crate::{clock::ClockBank, mixer::Channel, numbers::UnipolarFloat, tunnel::ArcSegment};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A look is a beam that is essentially the contents of an entire mixer.
/// All channel settings are preserved.
#[derive(Clone, Serialize, Deserialize)]
pub struct Look {
    pub channels: Vec<Channel>,
}

impl Look {
    pub fn from_channels(channels: Vec<Channel>) -> Self {
        Self { channels }
    }

    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        for channel in &mut self.channels {
            channel.update_state(delta_t, external_clocks);
        }
    }

    /// Draw all the Beams in this Look.
    ///
    /// The individual subchannels are unpacked and returned as a single channel of
    /// many arc segment commands.
    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
    ) -> Vec<ArcSegment> {
        let mut arcs = Vec::new();
        for channel in &self.channels {
            let mut rendered = channel.render(level, mask, external_clocks);
            arcs.append(&mut rendered);
        }
        arcs
    }
}
