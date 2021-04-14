use crate::beam::Beam;
use crate::ui::EmitStateChange as EmitShowStateChange;

/// Save beams in a grid store intended for simple access via APC button grid.
pub struct BeamStore {
    beams: Vec<Vec<Option<Beam>>>,
    state: State,
}

impl BeamStore {
    const N_ROWS: usize = 5;
    const COLS_PER_PAGE: usize = 8;

    pub fn new(n_pages: usize) -> Self {
        let mut rows = Vec::with_capacity(Self::N_ROWS);
        let n_cols = Self::COLS_PER_PAGE * n_pages;
        for _ in 0..Self::N_ROWS {
            rows.push(vec![None; n_cols]);
        }
        Self {
            beams: rows,
            state: State::Idle,
        }
    }

    pub fn put(&mut self, row: usize, col: usize, beam: Beam) {
        self.beams[row][col] = Some(beam);
    }

    pub fn clear(&mut self, row: usize, col: usize) {
        self.beams[row][col] = None;
    }

    pub fn get(&mut self, row: usize, col: usize) -> Option<Beam> {
        return self.beams[row][col].clone();
    }

    // /// Handle a control event.
    // /// Emit any state changes that have happened as a result of handling.
    // pub fn control<E: EmitStateChange>(
    //     &mut self,
    //     msg: ControlMessage,
    //     mixer_proxy: MixerProxy,
    //     emitter: &mut E,
    // ) {
    //     use ControlMessage::*;
    //     use State::*;
    //     match msg {
    //         GridButtonPress(row, col) => match self.state {
    //             Idle => {
    //                 if let Some(beam) = self.get(row, col) {
    //                     mixer_proxy.replace_current_beam(beam);
    //                 }
    //             }
    //         },
    //     }
    // }
}

enum State {
    Idle,
    BeamSave,
    LookSave,
    Delete,
    LookEdit,
}

enum ButtonContents {
    Empty,
    Tunnel,
    Look,
}
pub enum ControlMessage {
    GridButtonPress(usize, usize),
    BeamSave,
    LookSave,
    Delete,
    LookEdit,
}

pub enum StateChange {
    ButtonContents(ButtonContents),
}

// pub trait EmitStateChange {
//     fn emit_beam_store_state_change(&mut self, sc: StateChange);
// }

// impl<T: EmitShowStateChange> EmitStateChange for T {
//     fn emit_beam_store_state_change(&mut self, sc: StateChange) {
//         use crate::show::StateChange as ShowStateChange;
//         self.emit(ShowStateChange::BeamStore(sc))
//     }
// }
