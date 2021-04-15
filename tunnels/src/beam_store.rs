use crate::beam::Beam;

/// Save beams in a grid store intended for simple access via APC button grid.
pub struct BeamStore {
    beams: Vec<Vec<Option<Beam>>>,
}

impl BeamStore {
    pub const N_ROWS: usize = 5;
    pub const COLS_PER_PAGE: usize = 8;

    pub fn new(n_pages: usize) -> Self {
        let mut rows = Vec::with_capacity(Self::N_ROWS);
        let n_cols = Self::COLS_PER_PAGE * n_pages;
        for _ in 0..Self::N_ROWS {
            rows.push(vec![None; n_cols]);
        }
        Self { beams: rows }
    }

    pub fn put(&mut self, addr: BeamStoreAddr, beam: Option<Beam>) {
        self.beams[addr.row][addr.col] = beam;
    }

    pub fn get(&mut self, addr: BeamStoreAddr) -> Option<Beam> {
        return self.beams[addr.row][addr.col].clone();
    }

    pub fn items(&self) -> impl Iterator<Item = (BeamStoreAddr, &Option<Beam>)> {
        self.beams.iter().enumerate().flat_map(|(row, cols)| {
            cols.iter()
                .enumerate()
                .map(move |(col, beam)| (BeamStoreAddr { row, col }, beam))
        })
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct BeamStoreAddr {
    pub row: usize,
    pub col: usize,
}
