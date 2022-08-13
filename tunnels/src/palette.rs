use serde::{Deserialize, Serialize};
use tunnels_lib::color::Hsv;
use typed_index_derive::TypedIndex;

use crate::master_ui::EmitStateChange as EmitShowStateChange;

const MIN_PALETTE_SIZE: usize = 1;

#[derive(
    Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize, TypedIndex,
)]
#[typed_index(Hsv)]
pub struct ColorPaletteIdx(pub usize);

/// Store an array of colors that can be used by beams.
#[derive(Serialize, Deserialize, Clone)]
pub struct ColorPalette(Vec<Hsv>);

impl ColorPalette {
    pub fn new() -> Self {
        ColorPalette(vec![Hsv::BLACK; MIN_PALETTE_SIZE])
    }

    /// Return the color in the palette from the requested index.
    pub fn get(&self, index: ColorPaletteIdx) -> Option<Hsv> {
        self.0.get(index.0).copied()
    }

    /// Emit the current value of all controllable palette state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_palette_state_change(Contents(self.0.clone()));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            Set(sc) => self.handle_state_change(sc, emitter),
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            Contents(ref colors) => {
                self.0.clear();
                self.0.extend_from_slice(colors);
            }
        };
        emitter.emit_palette_state_change(sc);
    }
}

pub enum ControlMessage {
    Set(StateChange),
}

pub enum StateChange {
    Contents(Vec<Hsv>),
}

pub trait EmitStateChange {
    fn emit_palette_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_palette_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::ColorPalette(sc))
    }
}
