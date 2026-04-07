//! Audio subsystem — thin re-export layer over `tunnels_audio`.
//!
//! This module re-exports all public types from `tunnels_audio` and provides
//! the adapter that bridges the audio crate's `EmitStateChange` trait to
//! the show-level `EmitStateChange` trait.

// Re-export everything from tunnels_audio so existing `crate::audio::*`
// imports continue to work.
pub use tunnels_audio::*;

use crate::master_ui::EmitStateChange as EmitShowStateChange;

/// Newtype adapter that bridges the show's `EmitStateChange` to the audio
/// crate's `EmitStateChange`. Wraps a mutable reference to any show-level
/// emitter and routes audio state changes through `ShowStateChange::Audio`.
pub struct ShowEmitter<'a, T: EmitShowStateChange>(pub &'a mut T);

impl<T: EmitShowStateChange> tunnels_audio::EmitStateChange for ShowEmitter<'_, T> {
    fn emit_audio_state_change(&mut self, sc: tunnels_audio::StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.0.emit(ShowStateChange::Audio(sc))
    }
}
