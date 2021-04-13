use crate::{
    clock::ClockBank,
    look::Look,
    numbers::UnipolarFloat,
    tunnel::{ArcSegment, Tunnel},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Union type for all of the kinds of beams we can have.
/// Since we don't need beam to be very extensible, we will try this approach
/// instead of having to either treat beams as trait objects or store them in
/// disparate collections.
#[derive(Clone, Serialize, Deserialize)]
pub enum Beam {
    Tunnel(Tunnel),
    Look(Look),
}

impl Beam {
    pub fn update_state(&mut self, delta_t: Duration, external_clocks: &ClockBank) {
        match self {
            Self::Tunnel(t) => t.update_state(delta_t, external_clocks),
            Self::Look(l) => l.update_state(delta_t, external_clocks),
        }
    }

    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
    ) -> Vec<ArcSegment> {
        match self {
            Self::Tunnel(t) => t.render(level, mask, external_clocks),
            Self::Look(l) => l.render(level, mask, external_clocks),
        }
    }
}
