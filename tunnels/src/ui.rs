use std::collections::HashMap;

use crate::{
    beam::Beam,
    mixer::{LayerIdx, Mixer},
    show::{ControlMessage, StateChange},
    tunnel::AnimationIdx,
};

/// Manage stateful aspects of the UI.
/// Mediate between the input systems and the show data.
pub struct UI {
    current_layer: LayerIdx,
    current_animator: HashMap<LayerIdx, AnimationIdx>,
}

impl UI {
    pub fn new() -> Self {
        Self {
            current_layer: Default::default(),
            current_animator: HashMap::new(),
        }
    }

    pub fn handle_control_message<E: EmitStateChange>(
        &mut self,
        msg: ControlMessage,
        mixer: &mut Mixer,
        emitter: &mut E,
    ) {
        match msg {
            ControlMessage::Tunnel(tm) => match mixer.beam(self.current_layer) {
                Beam::Look(_) => (),
                Beam::Tunnel(t) => t.control(tm, emitter),
            },
            ControlMessage::Animation(am) => match mixer.beam(self.current_layer) {
                Beam::Look(_) => (),
                Beam::Tunnel(t) => {
                    let curr_anim = self
                        .current_animator
                        .get(&self.current_layer)
                        .unwrap_or(&AnimationIdx(0));

                    t.animation(*curr_anim).control(am, emitter);
                }
            },
        }
    }
}

pub trait EmitStateChange {
    fn emit(&mut self, sc: StateChange);
}
