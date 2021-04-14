use std::collections::HashMap;

use crate::{
    beam::Beam,
    look::Look,
    mixer::{ChannelIdx, Mixer},
    show::{ControlMessage, StateChange},
    tunnel::AnimationIdx,
};

/// Manage stateful aspects of the UI.
/// Mediate between the input systems and the show data.
pub struct UI {
    current_channel: ChannelIdx,
    current_animator: HashMap<ChannelIdx, AnimationIdx>,
}

impl UI {
    pub fn new() -> Self {
        Self {
            current_channel: Default::default(),
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
            ControlMessage::Tunnel(tm) => match mixer.beam(self.current_channel) {
                Beam::Look(_) => (),
                Beam::Tunnel(t) => t.control(tm, emitter),
            },
            ControlMessage::Animation(am) => match mixer.beam(self.current_channel) {
                Beam::Look(_) => (),
                Beam::Tunnel(t) => {
                    let curr_anim = self
                        .current_animator
                        .get(&self.current_channel)
                        .unwrap_or(&AnimationIdx(0));

                    t.animation(*curr_anim).control(am, emitter);
                }
            },
            ControlMessage::Mixer(mm) => {
                mixer.control(mm, emitter);
            }
        }
    }
}

pub trait EmitStateChange {
    fn emit(&mut self, sc: StateChange);
}

// /// Provide temporary and tightly scoped mutable access to the mixer.
// struct MixerProxy<'m> {
//     mixer: &'m mut Mixer,
//     current_layer: ChannelIdx,
// }
// impl<'m> MixerProxy<'m> {
//     /// Return a clone of the beam in the current channel.
//     fn get_current_beam(&mut self) -> Beam {
//         self.mixer.beam(self.current_layer).clone()
//     }
//     /// Replace the beam in the currently-selected channel.
//     fn replace_current_beam(&mut self, beam: Beam) {
//         self.mixer.put_beam(self.current_layer, beam);
//     }
//     /// Clone the contents of the mixer as a look.
//     fn generate_look(&self) -> Look {
//         self.mixer.as_look()
//     }
// }
