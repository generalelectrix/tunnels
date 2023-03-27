use crate::{beam::Beam, tunnel::Tunnel};
use serde::{Deserialize, Serialize};

/// Save beams in a grid store intended for simple access via APC button grid.
#[derive(Serialize, Deserialize)]
pub struct BeamStore {
    beams: Vec<Vec<Option<Beam>>>,
    n_pages: usize,
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

        // Start off with the default tunnel in the bottom-right corner.
        rows[4][7] = Some(Beam::Tunnel(Tunnel::default()));
        Self {
            beams: rows,
            n_pages,
        }
    }

    pub fn put(&mut self, addr: BeamStoreAddr, beam: Option<Beam>) {
        self.beams[addr.row][addr.col] = beam;
    }

    pub fn get(&mut self, addr: BeamStoreAddr) -> Option<Beam> {
        self.beams[addr.row][addr.col].clone()
    }

    pub fn items(&self) -> impl Iterator<Item = (BeamStoreAddr, &Option<Beam>)> {
        self.beams.iter().enumerate().flat_map(|(row, cols)| {
            cols.iter()
                .enumerate()
                .map(move |(col, beam)| (BeamStoreAddr { row, col }, beam))
        })
    }

    pub fn n_pages(&self) -> usize {
        self.n_pages
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct BeamStoreAddr {
    pub row: usize,
    pub col: usize,
}
