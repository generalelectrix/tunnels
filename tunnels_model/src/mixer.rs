use crate::layer::{Layer, LayerCollection, PaintMode};
use crate::render_context::RenderContext;
use crate::typed_index::typed_index;
use crate::{beam::Beam, look::Look, tunnel::Tunnel};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};
use tunnels_lib::number::UnipolarFloat;

/// The number of mixer channels on a single mixer page.
pub const MIXER_CHANNELS_PER_PAGE: usize = 8;

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

    /// Render the current state of the mixer for a single virtual video channel.
    ///
    /// Channels that are not routed to `video_channel` are not expanded at all,
    /// so the work is proportional to what that one video channel draws.
    pub fn render_video_channel(
        &self,
        video_channel: VideoChannel,
        ctx: RenderContext,
    ) -> LayerCollection {
        let mut video_out = Vec::new();
        // One buffer, reused across channels: a channel's layers are drained
        // into the output before the next channel renders into it.
        let mut rendered = Vec::new();
        for channel in &self.channels {
            if !channel.video_outs.contains(&video_channel) {
                continue;
            }
            rendered.clear();
            channel.render(UnipolarFloat::ONE, PaintMode::Normal, ctx, &mut rendered);
            for layer in rendered.drain(..) {
                if layer.is_empty() {
                    continue;
                }
                video_out.push(layer);
            }
        }
        video_out
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
            emit(ChannelStateChange::Mode(channel.mode));
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
            ToggleMode(mode) => {
                let current = self.channels[msg.channel].mode;
                let toggled = if current == mode {
                    PaintMode::Normal
                } else {
                    mode
                };
                self.handle_state_change(
                    StateChange {
                        channel: msg.channel,
                        change: ChannelStateChange::Mode(toggled),
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
            Mode(v) => self.channels[sc.channel].mode = v,
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
    pub mode: PaintMode,
    pub video_outs: BTreeSet<VideoChannel>,
}

impl Channel {
    fn new(beam: Beam) -> Self {
        let mut video_outs = BTreeSet::new();
        video_outs.insert(VideoChannel(0));
        Self {
            beam,
            level: UnipolarFloat::ZERO,
            bump: false,
            mode: PaintMode::Normal,
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
        mode: PaintMode,
        ctx: RenderContext,
        out: &mut Vec<Layer>,
    ) {
        let mut level: UnipolarFloat = if self.bump {
            UnipolarFloat::ONE
        } else {
            self.level
        };
        level *= level_scale;
        // if this channel is off, don't render at all
        if level == 0. {
            return;
        }
        self.beam.render(level, mode.over(self.mode), ctx, out);
    }
}

/// Index into a particular mixer channel.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ChannelIdx(pub usize);
typed_index!(ChannelIdx, Channel);

/// Index into a particular virtual video channel.
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct VideoChannel(pub usize);

#[derive(Debug)]
pub struct ControlMessage {
    pub channel: ChannelIdx,
    pub msg: ChannelControlMessage,
}
#[derive(Debug)]
pub enum ChannelControlMessage {
    Set(ChannelStateChange),
    /// Put the channel into this mode, or back to normal if it is there
    /// already.
    ///
    /// Naming the mode rather than stepping through them is what lets a
    /// control surface give each mode its own button and its own lamp: a
    /// button reads its channel's mode off its lamp, and one press of it
    /// reaches that mode from any other.
    ToggleMode(PaintMode),
    ToggleVideoChannel(VideoChannel),
}

#[derive(Debug)]
pub struct StateChange {
    pub channel: ChannelIdx,
    pub change: ChannelStateChange,
}
#[derive(Debug)]
pub enum ChannelStateChange {
    Level(UnipolarFloat),
    Bump(bool),
    Mode(PaintMode),
    VideoChannel((VideoChannel, bool)),
    ContainsLook(bool),
}

pub trait EmitStateChange {
    fn emit_mixer_state_change(&mut self, sc: StateChange);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::clock_bank::ClockBank;
    use crate::palette::ColorPalette;
    use crate::position_bank::PositionBank;
    use crate::tunnel::Tunnel;

    /// A channel taken off its upfader emits no layer, whatever mode it is in.
    ///
    /// This is what separates a gobo the operator has put away from one whose
    /// aperture has been animated shut: the first never reaches a renderer,
    /// and the second reaches it and blacks the frame. Without the level
    /// deciding it here, there would be no way to hold a gobo channel ready
    /// without blacking the video channel it is on.
    #[test]
    fn a_channel_off_at_the_upfader_emits_no_layer() {
        let clocks = ClockBank::default().as_static();
        let palette = ColorPalette::default();
        let positions = PositionBank::default();
        let ctx = RenderContext {
            clocks: &clocks,
            palette: &palette,
            positions: &positions,
            audio_envelope: UnipolarFloat::ZERO,
        };
        let channel = |level| Channel {
            beam: Beam::Tunnel(Tunnel::default()),
            level,
            bump: false,
            mode: PaintMode::Gobo,
            video_outs: BTreeSet::new(),
        };

        let mut out = Vec::new();
        channel(UnipolarFloat::ZERO).render(UnipolarFloat::ONE, PaintMode::Normal, ctx, &mut out);
        assert!(
            out.is_empty(),
            "a gobo channel at zero level emitted {} layers, and so would have \
             blacked the frame it was meant to be absent from",
            out.len()
        );

        channel(UnipolarFloat::ONE).render(UnipolarFloat::ONE, PaintMode::Normal, ctx, &mut out);
        assert_eq!(out.len(), 1, "a channel that is up emits its beam's layer");
        assert_eq!(
            out[0].mode(),
            PaintMode::Gobo,
            "the channel's own mode reaches the layer it emits"
        );
    }
}
