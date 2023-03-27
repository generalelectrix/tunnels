use crate::midi_controls::MIXER_CHANNELS_PER_PAGE;
use crate::palette::ColorPalette;
use crate::{beam::Beam, look::Look, tunnel::Tunnel};
use crate::{clock_bank::ClockBank, master_ui::EmitStateChange as EmitShowStateChange};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tunnels_lib::number::UnipolarFloat;
use tunnels_lib::{ArcSegment, LayerCollection};
use typed_index_derive::TypedIndex;

/// Holds a collection of beams in channels, and understands how they are mixed.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Mixer {
    channels: Vec<Channel>,
}

impl Mixer {
    pub const N_VIDEO_CHANNELS: usize = 8;

    pub fn new(n_pages: usize) -> Self {
        let n_channels = n_pages * MIXER_CHANNELS_PER_PAGE;
        Self {
            channels: (0..n_channels)
                .map(|_| Channel::new(Beam::Tunnel(Tunnel::default())))
                .collect(),
        }
    }

    /// Clone the contents of this mixer as a Look.
    pub fn as_look(&self) -> Look {
        Look::from_channels(self.channels.clone())
    }

    /// Clobber the state of this mixer with the provided look.
    pub fn set_look<E: EmitStateChange>(&mut self, look: Look, emitter: &mut E) {
        self.channels = look.channels;
        self.emit_state(emitter);
    }

    /// Update the state of all of the beams contained in this mixer.
    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        for channel in &mut self.channels {
            channel.update_state(delta_t, audio_envelope);
        }
    }

    pub fn beam(&mut self, channel: ChannelIdx) -> &mut Beam {
        &mut self.channels[channel].beam
    }

    pub fn channels(&mut self) -> impl Iterator<Item = &mut Channel> {
        self.channels.iter_mut()
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Render the current state of the mixer.
    /// Each inner vector represents one virtual video channel.
    pub fn render(
        &self,
        external_clocks: &ClockBank,
        color_palette: &ColorPalette,
        audio_envelope: UnipolarFloat,
    ) -> Vec<LayerCollection> {
        let mut video_outs = Vec::with_capacity(Self::N_VIDEO_CHANNELS);
        for _ in 0..Self::N_VIDEO_CHANNELS {
            video_outs.push(Vec::new());
        }
        for channel in &self.channels {
            let rendered_beam = channel.render(
                UnipolarFloat::ONE,
                false,
                external_clocks,
                color_palette,
                audio_envelope,
            );
            if rendered_beam.is_empty() {
                continue;
            }
            let rendered_ptr = Arc::new(rendered_beam);
            for video_chan in &channel.video_outs {
                video_outs[video_chan.0].push(rendered_ptr.clone());
            }
        }
        video_outs
    }

    /// Emit the current value of all controllable mixer state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        for (index, channel) in self.channels.iter().enumerate() {
            let mut emit = |csc| {
                emitter.emit_mixer_state_change(StateChange {
                    channel: ChannelIdx(index),
                    change: csc,
                })
            };
            emit(ChannelStateChange::Level(channel.level));
            emit(ChannelStateChange::Bump(channel.bump));
            emit(ChannelStateChange::Mask(channel.mask));
            emit(ChannelStateChange::ContainsLook(matches!(
                channel.beam,
                Beam::Look(_)
            )));
            for video_chan in 0..Self::N_VIDEO_CHANNELS {
                let vc = VideoChannel(video_chan);
                emit(ChannelStateChange::VideoChannel((
                    vc,
                    channel.video_outs.contains(&vc),
                )));
            }
        }
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ChannelControlMessage::*;
        match msg.msg {
            Set(sc) => self.handle_state_change(
                StateChange {
                    channel: msg.channel,
                    change: sc,
                },
                emitter,
            ),
            ToggleMask => {
                let toggled = !self.channels[msg.channel].mask;
                self.handle_state_change(
                    StateChange {
                        channel: msg.channel,
                        change: ChannelStateChange::Mask(toggled),
                    },
                    emitter,
                )
            }
            ToggleVideoChannel(vc) => {
                let toggled = !self.channels[msg.channel].video_outs.contains(&vc);
                self.handle_state_change(
                    StateChange {
                        channel: msg.channel,
                        change: ChannelStateChange::VideoChannel((vc, toggled)),
                    },
                    emitter,
                )
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use ChannelStateChange::*;
        match sc.change {
            Level(v) => self.channels[sc.channel].level = v,
            Bump(v) => self.channels[sc.channel].bump = v,
            Mask(v) => self.channels[sc.channel].mask = v,
            VideoChannel((vc, active)) => {
                if active {
                    self.channels[sc.channel].video_outs.insert(vc);
                } else {
                    self.channels[sc.channel].video_outs.remove(&vc);
                }
            }
            ContainsLook(_) => (),
        };
        emitter.emit_mixer_state_change(sc);
    }
}

/// The contents of a mixer channel.
///
/// By default, outputs to video feed 0.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Channel {
    pub beam: Beam,
    pub level: UnipolarFloat,
    pub bump: bool,
    pub mask: bool,
    pub video_outs: HashSet<VideoChannel>,
}

impl Channel {
    fn new(beam: Beam) -> Self {
        let mut video_outs = HashSet::new();
        video_outs.insert(VideoChannel(0));
        Self {
            beam,
            level: UnipolarFloat::ZERO,
            bump: false,
            mask: false,
            video_outs,
        }
    }

    /// Update the state of the beam in this channel.
    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        self.beam.update_state(delta_t, audio_envelope);
    }

    /// Render the beam in this channel.
    pub fn render(
        &self,
        level_scale: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
        color_palette: &ColorPalette,
        audio_envelope: UnipolarFloat,
    ) -> Vec<ArcSegment> {
        let mut level: UnipolarFloat = if self.bump {
            UnipolarFloat::ONE
        } else {
            self.level
        };
        level *= level_scale;
        // if this channel is off, don't render at all
        if level == 0. {
            return Vec::new();
        }
        self.beam.render(
            level,
            self.mask || mask,
            external_clocks,
            color_palette,
            audio_envelope,
        )
    }
}

/// Index into a particular mixer channel.
#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(Channel)]
pub struct ChannelIdx(pub usize);

/// Index into a particular virtual video channel.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct VideoChannel(pub usize);

pub struct ControlMessage {
    pub channel: ChannelIdx,
    pub msg: ChannelControlMessage,
}
pub enum ChannelControlMessage {
    Set(ChannelStateChange),
    ToggleMask,
    ToggleVideoChannel(VideoChannel),
}

pub struct StateChange {
    pub channel: ChannelIdx,
    pub change: ChannelStateChange,
}
pub enum ChannelStateChange {
    Level(UnipolarFloat),
    Bump(bool),
    Mask(bool),
    VideoChannel((VideoChannel, bool)),
    ContainsLook(bool),
}

pub trait EmitStateChange {
    fn emit_mixer_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_mixer_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Mixer(sc))
    }
}
