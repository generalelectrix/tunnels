//! The audio input's live condition, shared between the thread that measures
//! it and any thread that displays it.
use std::sync::atomic::{AtomicBool, Ordering};

use crate::processor::AtomicF32;

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
