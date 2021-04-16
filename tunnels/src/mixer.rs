use crate::master_ui::EmitStateChange as EmitShowStateChange;
use crate::{beam::Beam, clock::ClockBank, look::Look, numbers::UnipolarFloat, tunnel::Tunnel};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tunnels_lib::{ArcSegment, LayerCollection};
use typed_index_derive::TypedIndex;

/// Holds a collection of beams in channels, and understands how they are mixed.
#[derive(Clone, Serialize, Deserialize)]
pub struct Mixer {
    channels: Vec<Channel>,
}

impl Mixer {
    pub const N_VIDEO_CHANNELS: usize = 8;

    pub fn new(n_channels: usize) -> Self {
        let mut channels = Vec::with_capacity(n_channels);
        for i in 0..n_channels {
            channels.push(Channel::new(Beam::Tunnel(Tunnel::new())));
        }
        Self { channels }
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
    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        for channel in &mut self.channels {
            channel.update_state(delta_t, external_clocks);
        }
    }

    pub fn beam(&mut self, channel: ChannelIdx) -> &mut Beam {
        &mut self.channels[channel].beam
    }

    /// Render the current state of the mixer.
    /// Each inner vector represents one virtual video channel.
    pub fn render(&self, external_clocks: &ClockBank) -> Vec<LayerCollection> {
        let mut video_outs = Vec::with_capacity(Self::N_VIDEO_CHANNELS);
        for _ in 0..Self::N_VIDEO_CHANNELS {
            video_outs.push(Vec::new());
        }
        for channel in &self.channels {
            let rendered_beam = channel.render(UnipolarFloat(1.0), false, external_clocks);
            if rendered_beam.len() == 0 {
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
            emit(ChannelStateChange::ContainsLook(match channel.beam {
                Beam::Look(_) => true,
                _ => false,
            }));
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
        use ControlMessage::*;
        match msg {
            Channel((channel, chan_msg)) => match chan_msg {
                Set(sc) => self.handle_state_change(
                    StateChange {
                        channel: channel,
                        change: sc,
                    },
                    emitter,
                ),
                ToggleMask => {
                    let toggled = !self.channels[channel].mask;
                    self.handle_state_change(
                        StateChange {
                            channel: channel,
                            change: ChannelStateChange::Mask(toggled),
                        },
                        emitter,
                    )
                }
                ToggleVideoChannel(vc) => {
                    let toggled = !self.channels[channel].video_outs.contains(&vc);
                    self.handle_state_change(
                        StateChange {
                            channel: channel,
                            change: ChannelStateChange::VideoChannel((vc, toggled)),
                        },
                        emitter,
                    )
                }
                PutBeam(b) => {
                    let is_look = match *b {
                        Beam::Look(_) => true,
                        _ => false,
                    };
                    self.channels[channel].beam = *b;
                    emitter.emit_mixer_state_change(StateChange {
                        channel: channel,
                        change: ChannelStateChange::ContainsLook(is_look),
                    })
                }
            },
            SetLook(look) => {
                // clobber the entire mixer state
                self.channels = look.channels;
                // update the state given the new channel data
                self.emit_state(emitter);
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
#[derive(Clone, Serialize, Deserialize)]
pub struct Channel {
    beam: Beam,
    level: UnipolarFloat,
    bump: bool,
    mask: bool,
    video_outs: HashSet<VideoChannel>,
}

impl Channel {
    fn new(beam: Beam) -> Self {
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

    /// Update the state of the beam in this channel.
    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        self.beam.update_state(delta_t, external_clocks);
    }

    /// Render the beam in this channel.
    pub fn render(
        &self,
        level_scale: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
    ) -> Vec<ArcSegment> {
        let mut level: UnipolarFloat = if self.bump {
            UnipolarFloat(1.0)
        } else {
            self.level
        };
        // WTF Rust why don't you want to let me multiply my newtypes
        level = UnipolarFloat(level.0 * level_scale.0);
        // if this channel is off, don't render at all
        if level.0 == 0. {
            return Vec::new();
        }
        self.beam.render(level, self.mask || mask, external_clocks)
    }
}

/// Index into a particular mixer channel.
#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(Channel)]
pub struct ChannelIdx(pub usize);

impl Default for ChannelIdx {
    fn default() -> Self {
        ChannelIdx(0)
    }
}

/// Index into a particular virtual video channel.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct VideoChannel(pub usize);

pub enum ControlMessage {
    Channel((ChannelIdx, ChannelControlMessage)),
    SetLook(Look),
}

pub enum ChannelControlMessage {
    Set(ChannelStateChange),
    PutBeam(Box<Beam>),
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
