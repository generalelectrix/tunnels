//! Show state that every beam reads while rendering a frame.

use crate::clock_bank::StaticClockBank;
use crate::palette::ColorPalette;
use crate::position_bank::PositionBank;
use tunnels_lib::number::UnipolarFloat;

/// The state a beam resolves its parameters against for one frame.
///
/// These values are fixed for the duration of a frame and are read, never
/// written, so they travel together through every level of the beam tree.
#[derive(Clone, Copy)]
pub struct RenderContext<'a> {
    pub clocks: &'a StaticClockBank,
    pub palette: &'a ColorPalette,
    pub positions: &'a PositionBank,
    pub audio_envelope: UnipolarFloat,
}
