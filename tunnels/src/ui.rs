use std::collections::HashMap;

use crate::{
    animation::Animation,
    beam::Beam,
    beam_store::BeamStore,
    mixer::{ChannelIdx, Mixer},
    show::{ControlMessage as ShowControlMessage, StateChange as ShowStateChange},
    tunnel::AnimationIdx,
};

/// Manage stateful aspects of the UI.
/// Mediate between the input systems and the show data.
pub struct MasterUI {
    current_channel: ChannelIdx,
    /// Index which animation is selected for the channel corresponding to the
    /// associated index.
    /// Enables stable animation selection when jumping between beams.
    current_animation_for_channel: Vec<AnimationIdx>,
    animation_clipboard: Animation,
    beam_store: BeamStore,
}

impl MasterUI {
    pub fn new(n_mixer_channels: usize, n_mixer_pages: usize) -> Self {
        Self {
            current_channel: Default::default(),
            current_animation_for_channel: vec![AnimationIdx(0); n_mixer_channels],
            animation_clipboard: Animation::new(),
            beam_store: BeamStore::new(n_mixer_pages),
        }
    }

    fn current_beam<'m>(&self, mixer: &'m mut Mixer) -> &'m mut Beam {
        mixer.beam(self.current_channel)
    }

    fn current_animation<'m>(&self, mixer: &'m mut Mixer) -> Option<&'m mut Animation> {
        match self.current_beam(mixer) {
            Beam::Look(_) => None,
            Beam::Tunnel(t) => Some(t.animation(self.current_animation_idx())),
        }
    }

    fn current_animation_idx(&self) -> AnimationIdx {
        self.current_animation_for_channel[self.current_channel.0]
    }

    pub fn handle_control_message<E: EmitStateChange>(
        &mut self,
        msg: ShowControlMessage,
        mixer: &mut Mixer,
        emitter: &mut E,
    ) {
        match msg {
            ShowControlMessage::Tunnel(tm) => match self.current_beam(mixer) {
                Beam::Look(_) => (),
                Beam::Tunnel(t) => t.control(tm, emitter),
            },
            ShowControlMessage::Animation(am) => {
                if let Some(a) = self.current_animation(mixer) {
                    a.control(am, emitter);
                }
            }
            ShowControlMessage::Mixer(mm) => {
                mixer.control(mm, emitter);
            }
            ShowControlMessage::MasterUI(uim) => self.control(uim, mixer, emitter),
        }
    }

    /// Emit all controllable state.
    fn emit_state<E: EmitStateChange>(&self, mixer: &mut Mixer, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_master_ui_state_change(Channel(self.current_channel));
        // self.beam_store.emit_state(emitter);
        self.emit_current_channel_state(mixer, emitter);
    }

    /// Emit state for the active animator.
    fn emit_animator_state<E: EmitStateChange>(&self, mixer: &mut Mixer, emitter: &mut E) {
        if let Some(a) = self.current_animation(mixer) {
            a.emit_state(emitter);
        }
        emitter.emit_master_ui_state_change(StateChange::Animation(self.current_animation_idx()));
    }

    /// Emit state for the active beam and animator.
    fn emit_current_channel_state<E: EmitStateChange>(&self, mixer: &mut Mixer, emitter: &mut E) {
        // Emit state for the beam in the current channel.
        // Do nothing if the beam is a look.
        // FIXME: we should do something nice like turn all the UI LEDs
        // off when the current channel is a look.
        match self.current_beam(mixer) {
            Beam::Look(_) => (),
            Beam::Tunnel(t) => {
                t.emit_state(emitter);
            }
        }
        self.emit_animator_state(mixer, emitter);
    }

    fn control<E: EmitStateChange>(
        &mut self,
        msg: ControlMessage,
        mixer: &mut Mixer,
        emitter: &mut E,
    ) {
        use ControlMessage::*;

        match msg {
            Set(sc) => self.handle_state_change(sc, mixer, emitter),
            AnimationCopy => {
                if let Some(a) = self.current_animation(mixer) {
                    self.animation_clipboard = a.clone();
                }
            }
            AnimationPaste => {
                if let Some(a) = self.current_animation(mixer) {
                    *a = self.animation_clipboard.clone();
                }
                self.emit_animator_state(mixer, emitter);
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(
        &mut self,
        sc: StateChange,
        mixer: &mut Mixer,
        emitter: &mut E,
    ) {
        use StateChange::*;
        match sc {
            Channel(chan) => {
                // No action if we already have this channel selected.
                if chan == self.current_channel {
                    return;
                }
                self.current_channel = chan;
                self.emit_current_channel_state(mixer, emitter);
                emitter.emit_master_ui_state_change(sc);
            }
            Animation(a) => {
                self.current_animation_for_channel[self.current_channel.0] = a;
                self.emit_animator_state(mixer, emitter);
            }
        }
    }
}

pub trait EmitStateChange {
    fn emit(&mut self, sc: ShowStateChange);
}
pub trait EmitMasterUIStateChange {
    fn emit_master_ui_state_change(&mut self, sc: StateChange);
}

impl<T: EmitStateChange> EmitMasterUIStateChange for T {
    fn emit_master_ui_state_change(&mut self, sc: StateChange) {
        self.emit(ShowStateChange::MasterUI(sc))
    }
}

pub enum ControlMessage {
    Set(StateChange),
    AnimationCopy,
    AnimationPaste,
}

pub enum StateChange {
    Channel(ChannelIdx),
    Animation(AnimationIdx),
}
