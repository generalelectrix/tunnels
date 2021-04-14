use crate::beam::Beam;

/// Save beams in a grid store intended for simple access via APC button grid.
pub struct BeamStore {
    beams: Vec<Vec<Option<Beam>>>,
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
        Self { beams: rows }
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
}
