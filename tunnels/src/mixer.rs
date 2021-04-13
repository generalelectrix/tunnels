use crate::{
    clock::ClockBank,
    numbers::UnipolarFloat,
    tunnel::{ArcSegment, Tunnel},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, rc::Rc, time::Duration};

/// Index into a particular mixer channel.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LayerIdx(usize);
/// The contents of a mixer channel.
///
/// By default, outputs to video feed 0.
#[derive(Clone, Serialize, Deserialize)]
pub struct Layer {
    beam: Tunnel,
    level: UnipolarFloat,
    bump: bool,
    mask: bool,
    video_outs: HashSet<VideoChannel>,
}

impl Layer {
    fn new(beam: Tunnel) -> Self {
        let mut video_outs = HashSet::new();
        video_outs.insert(VideoChannel(0));
        Self {
            beam,
            level: UnipolarFloat(0.0),
            bump: false,
            mask: false,
            video_outs,
        }
    }

    /// Update the state of the beam in this layer.
    fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        self.beam.update_state(delta_t, external_clocks);
    }
}

/// Holds a collection of beams in layers, and understands how they are mixed.
#[derive(Clone, Serialize, Deserialize)]
pub struct Mixer {
    layers: Vec<Layer>,
}

impl Mixer {
    const N_VIDEO_CHANNELS: usize = 8;

    pub fn new(n_layers: usize) -> Self {
        let mut layers = Vec::with_capacity(n_layers);
        for i in 0..n_layers {
            layers.push(Layer::new(Tunnel::new()));
        }
        Self { layers }
    }

    /// Update the state of all of the beams contained in this mixer.
    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        for layer in &mut self.layers {
            layer.update_state(delta_t, external_clocks);
        }
    }

    pub fn put_beam_in_layer(&mut self, layer: LayerIdx, beam: Tunnel) {
        self.layers[layer.0].beam = beam;
    }

    pub fn set_level(&mut self, layer: LayerIdx, level: UnipolarFloat) {
        self.layers[layer.0].level = level;
    }

    pub fn set_bump(&mut self, layer: LayerIdx, bump: bool) {
        self.layers[layer.0].bump = bump;
    }

    /// Toggle the masking state of the selected layer.
    /// Return the new state.
    pub fn toggle_mask(&mut self, layer: LayerIdx) -> bool {
        let toggled = !self.layers[layer.0].mask;
        self.layers[layer.0].mask = toggled;
        toggled
    }

    /// Toggle the whether layer is drawn to video channel.
    ///
    /// Return the new state of display of this channel.
    pub fn toggle_video_channel(&mut self, layer: LayerIdx, channel: VideoChannel) -> bool {
        let outs = &mut self.layers[layer.0].video_outs;
        match outs.take(&channel) {
            // channel was active and is now inactive
            Some(_) => false,
            // channel was inactive and should be made active
            None => {
                outs.insert(channel);
                true
            }
        }
    }

    /// Render the current state of the mixer.
    /// Each inner vector represents one virtual video channel.
    pub fn render(&self, external_clocks: &ClockBank) -> Vec<Vec<Rc<Vec<ArcSegment>>>> {
        let mut video_outs = Vec::with_capacity(Self::N_VIDEO_CHANNELS);
        for _ in 0..Self::N_VIDEO_CHANNELS {
            video_outs.push(Vec::new());
        }
        for layer in &self.layers {
            let level = if layer.bump {
                UnipolarFloat(1.0)
            } else {
                layer.level
            };
            // if this layer is off, don't render it at all
            if level.0 == 0. {
                continue;
            }
            let rendered_beam = Rc::new(layer.beam.render(level, layer.mask, external_clocks));
            for video_chan in &layer.video_outs {
                video_outs[video_chan.0].push(rendered_beam.clone());
            }
        }
        video_outs
    }
}

/// Index into a particular virtual video channel.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct VideoChannel(usize);
