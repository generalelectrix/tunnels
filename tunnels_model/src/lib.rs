//! The show model and its render.
//!
//! Everything here is a pure description of what a show looks like, plus the
//! expansion of that description into drawable geometry. Nothing in this crate
//! talks to MIDI, OSC, the network, or a GUI, so it can be built into a render
//! client as well as into the console.

pub mod animation;
pub mod animation_target;
pub mod beam;
pub mod clock;
pub mod clock_bank;
mod look;
pub mod mixer;
pub mod palette;
pub mod position_bank;
pub mod render_context;
pub mod show_frame;
pub mod tunnel;
mod typed_index;
mod waveforms;
