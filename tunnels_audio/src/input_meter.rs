//! The audio input's live condition, shared between the thread that measures
//! it and any thread that displays it.
use std::sync::atomic::{AtomicBool, Ordering};

use crate::processor::AtomicF32;

/// The resolution the input trim is displayed at, in dB.
pub const TRIM_DISPLAY_STEP_DB: f32 = 0.5;

/// The automatic trim's current gain and whether the clip indicator is lit.
///
/// One thread writes it and any number read it, without locks or allocation.
/// Each field is current on its own; the two are not read as a pair.
#[derive(Debug)]
pub struct InputMeter {
    /// The trim's gain in dB.
    trim_db: AtomicF32,
    clip_lit: AtomicBool,
}

impl Default for InputMeter {
    /// Unity trim, clip indicator dark.
    fn default() -> Self {
        Self {
            trim_db: AtomicF32::new(0.0),
            clip_lit: AtomicBool::new(false),
        }
    }
}

impl InputMeter {
    /// The gain the automatic trim is applying, in dB.
    pub fn trim_db(&self) -> f32 {
        self.trim_db.get()
    }

    /// The gain the automatic trim is applying, in dB, to the nearest
    /// [`TRIM_DISPLAY_STEP_DB`], never negative zero.
    pub fn displayed_trim_db(&self) -> f32 {
        (self.trim_db() / TRIM_DISPLAY_STEP_DB).round() * TRIM_DISPLAY_STEP_DB + 0.0
    }

    /// Whether the clip indicator is lit.
    pub fn clip_lit(&self) -> bool {
        self.clip_lit.load(Ordering::Relaxed)
    }

    /// Record the trim's gain in dB and whether the clip indicator is lit.
    pub fn set(&self, trim_db: f32, clip_lit: bool) {
        self.trim_db.set(trim_db);
        self.clip_lit.store(clip_lit, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_is_displayed_to_the_nearest_half_db() {
        let meter = InputMeter::default();
        let displayed = |trim_db| {
            meter.set(trim_db, false);
            meter.displayed_trim_db()
        };
        assert_eq!(displayed(3.1), 3.0);
        assert_eq!(displayed(3.4), 3.5);
        assert_eq!(displayed(-9.8), -10.0);
        assert_eq!(
            format!("{:+.1}", displayed(-0.2)),
            "+0.0",
            "a trim that rounds to zero shows no sign of having been negative"
        );
    }
}
