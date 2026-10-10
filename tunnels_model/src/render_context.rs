//! Show state that every beam reads while rendering a frame.

use crate::clock_bank::StaticClockBank;
use crate::palette::ColorPalette;
use crate::position_bank::PositionBank;
use crate::spectrum::SpectrumTables;
use tunnels_lib::audio::AudioState;

/// The state a beam resolves its parameters against for one frame.
///
/// These values are fixed for the duration of a frame and are read, never
/// written, so they travel together through every level of the beam tree.
///
/// `'f` is the spectrum's borrow alone, so anything that keeps hold of the
/// spectrum is bound by it and not by the other borrows.
#[derive(Clone, Copy)]
pub struct RenderContext<'a, 'f> {
    pub clocks: &'a StaticClockBank,
    pub palette: &'a ColorPalette,
    pub positions: &'a PositionBank,
    pub audio: &'a AudioState,
    pub spectrum: &'f SpectrumTables,
}
