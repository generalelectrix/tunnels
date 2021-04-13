use crate::{
    beam::Beam,
    mixer::{LayerIdx, Mixer},
    show::{ControlMessage, StateChange},
};

/// Manage stateful aspects of the UI.
/// Mediate between the input systems and the show data.
pub struct UI {
    current_layer: LayerIdx,
}

impl UI {
    pub fn new() -> Self {
        Self {
            current_layer: Default::default(),
        }
    }

    pub fn handle_control_message<E>(&mut self, msg: ControlMessage, mixer: &mut Mixer, emit: E)
    where
        E: Fn(StateChange),
    {
        match msg {
            ControlMessage::Tunnel(tm) => match mixer.beam(self.current_layer) {
                Beam::Tunnel(t) => t.control(tm, |sc| emit(StateChange::Tunnel(sc))),
                Beam::Look(_) => (),
            },
        }
    }
}
