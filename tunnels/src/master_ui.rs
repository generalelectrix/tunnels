use crate::{
    animation::Animation,
    audio::AudioInput,
    beam::Beam,
    beam_store::{BeamStore, BeamStoreAddr},
    clock_bank::ClockBank,
    midi_controls::MIXER_CHANNELS_PER_PAGE,
    mixer::{ChannelIdx, Mixer},
    palette::ColorPalette,
    show::{ControlMessage as ShowControlMessage, StateChange as ShowStateChange},
    tunnel::AnimationIdx,
};

use serde::{Deserialize, Serialize};

/// Manage stateful aspects of the UI.
/// Mediate between the input systems and the show data.
#[derive(Serialize, Deserialize)]
pub struct MasterUI {
    current_channel: ChannelIdx,
    /// Index which animation is selected for the channel corresponding to the
    /// associated index.
    /// Enables stable animation selection when jumping between beams.
    current_animation_for_channel: Vec<AnimationIdx>,
    animation_clipboard: Animation,
    beam_store: BeamStore,
    beam_store_state: BeamStoreState,
}

impl MasterUI {
    pub fn new(n_mixer_pages: usize) -> Self {
        Self {
            current_channel: Default::default(),
            current_animation_for_channel: vec![
                AnimationIdx(0);
                n_mixer_pages * MIXER_CHANNELS_PER_PAGE
            ],
            animation_clipboard: Animation::new(),
            beam_store: BeamStore::new(n_mixer_pages),
            beam_store_state: BeamStoreState::Idle,
        }
    }

    pub fn n_pages(&self) -> usize {
        self.beam_store.n_pages()
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
        clocks: &mut ClockBank,
        color_palette: &mut ColorPalette,
        audio_input: &mut AudioInput,
        emitter: &mut E,
    ) {
        use ShowControlMessage::*;
        match msg {
            Tunnel(tm) => match self.current_beam(mixer) {
                Beam::Look(_) => (),
                Beam::Tunnel(t) => t.control(tm, emitter),
            },
            Animation(am) => {
                if let Some(a) = self.current_animation(mixer) {
                    a.control(am, emitter);
                }
            }
            Mixer(mm) => {
                mixer.control(mm, emitter);
            }
            Clock(cm) => {
                clocks.control(cm, emitter);
            }
            ColorPalette(cm) => {
                color_palette.control(cm, emitter);
            }
            Audio(cm) => {
                audio_input.control(cm, emitter);
            }
            MasterUI(uim) => self.control(uim, mixer, emitter),
        }
    }

    /// Emit all controllable state.
    pub fn emit_state<E: EmitStateChange>(
        &self,
        mixer: &mut Mixer,
        clocks: &mut ClockBank,
        color_palette: &mut ColorPalette,
        audio_input: &mut AudioInput,
        emitter: &mut E,
    ) {
        emitter.emit_master_ui_state_change(StateChange::Channel(self.current_channel));
        self.emit_beam_store_state(emitter);
        self.emit_current_channel_state(mixer, emitter);
        mixer.emit_state(emitter);
        clocks.emit_state(emitter);
        color_palette.emit_state(emitter);
        audio_input.emit_state(emitter);
    }

    /// Emit state for the beam store.
    fn emit_beam_store_state<E: EmitStateChange>(&self, emitter: &mut E) {
        for (addr, beam) in self.beam_store.items() {
            emitter.emit_master_ui_state_change(StateChange::BeamButton((
                addr,
                BeamButtonState::from_beam(beam),
            )));
        }
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

    fn set_beam_store_state<E: EmitStateChange>(&mut self, state: BeamStoreState, emitter: &mut E) {
        self.beam_store_state = state;
        emitter.emit_master_ui_state_change(StateChange::BeamStoreState(state));
    }

    fn put_beam_in_store<E: EmitStateChange>(
        &mut self,
        addr: BeamStoreAddr,
        beam: Option<Beam>,
        emitter: &mut E,
    ) {
        let button_state = BeamButtonState::from_beam(&beam);
        self.beam_store.put(addr, beam);
        emitter.emit_master_ui_state_change(StateChange::BeamButton((addr, button_state)));
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
            BeamGridButtonPress(addr) => self.handle_beam_grid_button_press(addr, mixer, emitter),
        }
    }

    fn handle_beam_grid_button_press<E: EmitStateChange>(
        &mut self,
        addr: BeamStoreAddr,
        mixer: &mut Mixer,
        emitter: &mut E,
    ) {
        use BeamStoreState::*;
        match self.beam_store_state {
            Idle => {
                // Request to replace the beam in the current mixer with
                // the beam in this button.
                if let Some(beam) = self.beam_store.get(addr) {
                    *self.current_beam(mixer) = beam;
                    self.emit_current_channel_state(mixer, emitter);
                }
            }
            BeamSave => {
                // Dump the current beam into the selected slot.
                self.put_beam_in_store(addr, Some(self.current_beam(mixer).clone()), emitter);
                self.set_beam_store_state(Idle, emitter);
            }
            LookSave => {
                // Dump the whole mixer state.
                self.put_beam_in_store(addr, Some(Beam::Look(mixer.as_look())), emitter);
                self.set_beam_store_state(Idle, emitter);
            }
            Delete => {
                self.put_beam_in_store(addr, None, emitter);
                self.set_beam_store_state(Idle, emitter);
            }
            LookEdit => {
                // If the beam in the requested slot is a look, explode
                // it into the mixer.
                if let Some(Beam::Look(look)) = self.beam_store.get(addr) {
                    mixer.set_look(look, emitter);
                    self.emit_current_channel_state(mixer, emitter);
                    self.set_beam_store_state(Idle, emitter);
                }
            }
        }
    }

    fn handle_state_change<E: EmitStateChange>(
        &mut self,
        sc: StateChange,
        mixer: &mut Mixer,
        emitter: &mut E,
    ) {
        match sc {
            StateChange::Channel(chan) => {
                // No action if we already have this channel selected.
                if chan == self.current_channel {
                    return;
                }
                self.current_channel = chan;
                self.emit_current_channel_state(mixer, emitter);
                emitter.emit_master_ui_state_change(sc);
            }
            StateChange::Animation(a) => {
                self.current_animation_for_channel[self.current_channel.0] = a;
                self.emit_animator_state(mixer, emitter);
            }
            StateChange::BeamStoreState(state) => {
                self.set_beam_store_state(
                    if self.beam_store_state == state {
                        BeamStoreState::Idle
                    } else {
                        state
                    },
                    emitter,
                );
            }
            // Output only.
            StateChange::BeamButton(_) => (),
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
    BeamGridButtonPress(BeamStoreAddr),
}

pub enum StateChange {
    Channel(ChannelIdx),
    Animation(AnimationIdx),
    BeamButton((BeamStoreAddr, BeamButtonState)),
    // Note that when provided as a control, this acts like a toggle.
    // One press sets the mode, a second press sets back to idle.
    BeamStoreState(BeamStoreState),
}

#[derive(Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum BeamStoreState {
    Idle,
    BeamSave,
    LookSave,
    Delete,
    LookEdit,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum BeamButtonState {
    Empty,
    Beam,
    Look,
}

impl BeamButtonState {
    pub fn from_beam(beam: &Option<Beam>) -> Self {
        match beam {
            Some(Beam::Tunnel(_)) => Self::Beam,
            Some(Beam::Look(_)) => Self::Look,
            None => Self::Empty,
        }
    }
}
