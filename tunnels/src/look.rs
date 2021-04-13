use crate::{clock::ClockBank, mixer::Layer, numbers::UnipolarFloat, tunnel::ArcSegment};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A look is a beam that is essentially the contents of an entire mixer.
/// All layer settings are preserved.
#[derive(Clone, Serialize, Deserialize)]
pub struct Look {
    layers: Vec<Layer>,
}

impl Look {
    pub fn from_layers(layers: Vec<Layer>) -> Self {
        Self { layers }
    }

    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        for layer in &mut self.layers {
            layer.update_state(delta_t, external_clocks);
        }
    }

    /// Draw all the Beams in this Look.
    ///
    /// The individual sublayers are unpacked and returned as a single layer of
    /// many arc segment commands.
    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
    ) -> Vec<ArcSegment> {
        let mut arcs = Vec::new();
        for layer in &self.layers {
            let mut rendered = layer.render(level, mask, external_clocks);
            arcs.append(&mut rendered);
        }
        arcs
    }
}
