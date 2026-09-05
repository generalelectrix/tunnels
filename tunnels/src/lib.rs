pub mod animation_visualizer;
pub mod audio;
mod beam_store;
pub mod clock_server;
pub mod control;
pub mod gui_state;
mod master_ui;
pub mod midi;
pub mod midi_controls;
pub mod osc;
mod send;
pub mod show;
pub mod test_mode;

// The show model and its render live in `tunnels_model` so that a render client
// can depend on them without pulling in MIDI, OSC, audio or the GUI. They are
// re-exported here at their original paths so that both this crate and its
// downstream consumers see no change.
pub use tunnels_model::{animation, clock, clock_bank, render_context, tunnel};
pub(crate) use tunnels_model::{animation_target, beam, mixer, palette, position_bank};
