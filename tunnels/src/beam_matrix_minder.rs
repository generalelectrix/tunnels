use crate::tunnel::Tunnel;

enum UIState {
    Idle,
    BeamSave,
    LookSave,
    Delete,
    LookEdit,
}

/// Dealing with the matrix of APC40 buttons used to store beams.
pub struct BeamMatrixMinder {
    beams: Vec<Vec<Option<Tunnel>>>,
}

impl BeamMatrixMinder {
    const N_ROWS: usize = 5;
    const COLS_PER_PAGE: usize = 8;

    pub fn new(n_pages: usize) -> Self {
        let mut rows = Vec::with_capacity(Self::N_ROWS);
        let n_cols = Self::COLS_PER_PAGE * n_pages;
        for _ in 0..Self::N_ROWS {
            rows.push(vec![None; n_cols]);
        }
        Self { beams: rows }
    }

    pub fn put(&mut self, row: usize, col: usize, beam: Tunnel) {
        self.beams[row][col] = Some(beam);
    }

    pub fn clear(&mut self, row: usize, col: usize) {
        self.beams[row][col] = None;
    }

    pub fn get(&mut self, row: usize, col: usize) -> Option<Tunnel> {
        return self.beams[row][col].clone();
    }
}
