use crate::palette::ColorPalette;
use crate::position_bank::PositionBank;
use crate::{clock_bank::ClockBank, look::Look, tunnel::Tunnel};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;
use tunnels_lib::ArcSegment;

/// Union type for all of the kinds of beams we can have.
/// Since we don't need beam to be very extensible, we will try this approach
/// instead of having to either treat beams as trait objects or store them in
/// disparate collections.
#[derive(Clone, Serialize, Deserialize, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Beam {
    Tunnel(Tunnel),
    Look(Look),
}

impl Beam {
    pub fn update_state(&mut self, delta_t: Duration, audio_envelope: UnipolarFloat) {
        match self {
            Self::Tunnel(t) => t.update_state(delta_t, audio_envelope),
            Self::Look(l) => l.update_state(delta_t, audio_envelope),
        }
    }

    pub fn render(
        &self,
        level: UnipolarFloat,
        mask: bool,
        external_clocks: &ClockBank,
        color_palette: &ColorPalette,
        positions: &PositionBank,
        audio_envelope: UnipolarFloat,
    ) -> Vec<ArcSegment> {
        match self {
            Self::Tunnel(t) => t.render(
                level,
                mask,
                external_clocks,
                color_palette,
                positions,
                audio_envelope,
            ),
            Self::Look(l) => l.render(
                level,
                mask,
                external_clocks,
                color_palette,
                positions,
                audio_envelope,
            ),
        }
    }
}
