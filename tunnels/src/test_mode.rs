use crate::animation_target::AnimationTarget;
use crate::master_ui::EmitStateChange;
use crate::{
    animation::{Animation, StateChange as AnimationStateChange, Waveform},
    beam::Beam,
    mixer::{Channel, Mixer, VideoChannel},
    show::StateChange,
    tunnel::{StateChange as TunnelStateChange, Tunnel},
};
use tunnels_lib::number::{BipolarFloat, UnipolarFloat};

pub type TestModeSetup = fn(usize, usize, &mut Channel);

/// A basic test mode outputting a slowly moving tunnel on each channel.
/// Each channel is given a slightly different color.
pub fn all_video_outputs(_: usize, i: usize, channel: &mut Channel) {
    channel.level = UnipolarFloat::ONE;
    channel.video_outs.clear();
    channel.video_outs.insert(VideoChannel(i));

    if let Beam::Tunnel(ref mut tunnel) = channel.beam {
        use TunnelStateChange::*;

        set_tunnel_state(tunnel, ColorSaturation(UnipolarFloat::ONE));
        set_tunnel_state(tunnel, MarqueeSpeed(BipolarFloat::new(0.1)));
        set_tunnel_state(
            tunnel,
            ColorCenter(UnipolarFloat::new(
                (i as f64 / Mixer::N_VIDEO_CHANNELS as f64) % 1.0,
            )),
        );
    }
}

/// A test mode designed to load the console as hard possible.
pub fn stress(channel_count: usize, i: usize, channel: &mut Channel) {
    channel.level = UnipolarFloat::ONE;

    if let Beam::Tunnel(ref mut tunnel) = channel.beam {
        use TunnelStateChange::*;

        set_tunnel_state(tunnel, ColorWidth(UnipolarFloat::new(0.25)));
        set_tunnel_state(tunnel, ColorSpread(UnipolarFloat::ONE));
        set_tunnel_state(tunnel, ColorSaturation(UnipolarFloat::new(0.25)));
        set_tunnel_state(
            tunnel,
            MarqueeSpeed(BipolarFloat::new(
                -1.0 + (2.0 * i as f64 / channel_count as f64),
            )),
        );
        set_tunnel_state(tunnel, Blacking(BipolarFloat::ZERO));

        for (i, anim) in tunnel.animations().enumerate() {
            set_animation_state(
                &mut anim.animation,
                AnimationStateChange::Waveform(match i % 4 {
                    0 => Waveform::Sine,
                    1 => Waveform::Triangle,
                    2 => Waveform::Square,
                    _ => Waveform::Sawtooth,
                }),
            );
            set_animation_state(
                &mut anim.animation,
                AnimationStateChange::Speed(BipolarFloat::new(i as f64 / 3.0)),
            );
            set_animation_state(
                &mut anim.animation,
                AnimationStateChange::Size(UnipolarFloat::new(0.5)),
            );
            anim.target = AnimationTarget::Thickness;
            set_animation_state(&mut anim.animation, AnimationStateChange::NPeriods(3));
        }
    }
}

struct DummyEmitter;

impl EmitStateChange for DummyEmitter {
    fn emit(&mut self, _: StateChange) {}
}

fn set_tunnel_state(tunnel: &mut Tunnel, state: TunnelStateChange) {
    use crate::tunnel::ControlMessage;
    tunnel.control(ControlMessage::Set(state), &mut DummyEmitter);
}

fn set_animation_state(animation: &mut Animation, state: AnimationStateChange) {
    use crate::animation::ControlMessage;
    animation.control(ControlMessage::Set(state), &mut DummyEmitter);
}
