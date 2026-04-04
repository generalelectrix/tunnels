use crate::typed_index::typed_index;
use serde::{Deserialize, Serialize};

const MIN_POSITION_COUNT: usize = 1;

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Default)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

pub type Positions = Vec<Position>;

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct PositionIdx(pub usize);
typed_index!(PositionIdx, Position);

/// Store an array of positions that can be used by beams.
#[derive(Serialize, Deserialize, Clone)]
pub struct PositionBank(Vec<Position>);

impl PositionBank {
    /// Return the position from the requested index.
    pub fn get(&self, index: PositionIdx) -> Option<Position> {
        self.0.get(index.0).copied()
    }

    /// Handle a control event.
    /// No state is emitted as a result of this action.
    pub fn control(&mut self, positions: Positions) {
        self.0 = positions;
    }
}

impl Default for PositionBank {
    fn default() -> Self {
        PositionBank(vec![Position::default(); MIN_POSITION_COUNT])
    }
}

pub type ControlMessage = Positions;
