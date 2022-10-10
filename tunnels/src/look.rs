use crate::palette::ColorPalette;
use crate::{clock_bank::ClockBank, mixer::Channel};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;
use tunnels_lib::ArcSegment;

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
    /// The individual subchannels are unpacked and returned as a single channel of
    /// many arc segment commands.
    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
        color_palette: &ColorPalette,
        audio_envelope: UnipolarFloat,
    ) -> Vec<ArcSegment> {
        let mut arcs = Vec::new();
        for channel in &self.channels {
            let mut rendered =
                channel.render(level, mask, external_clocks, color_palette, audio_envelope);
            arcs.append(&mut rendered);
        }
        arcs
    }
}
