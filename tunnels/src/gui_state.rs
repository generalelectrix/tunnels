use std::sync::Arc;

use arc_swap::ArcSwap;
use midi_harness::SlotStatus;
use tunnels_audio::AudioSnapshot;
use tunnels_lib::notified::{Notified, NotifiedAtomicBool};
use tunnels_lib::repaint::RepaintSignal;

use crate::animation_visualizer::AnimationSnapshot;

bitflags::bitflags! {
    /// GUI state domains that may need re-snapshotting after a control event.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GuiDirty: u8 {
        const CLEAN         = 0b0000_0000;
        const MIDI_SLOTS    = 0b0000_0001;
        const AUDIO         = 0b0000_0010;
        const CLOCK_SERVICE = 0b0000_0100;
    }
}

/// Shared state readable by the GUI. Writes that should wake an idle GUI use
/// `Notified`; high-frequency streaming fields use raw `ArcSwap` because the
/// consumer is already repainting continuously.
pub struct GuiState {
    pub midi_slots: Notified<Vec<SlotStatus>>,
    pub audio_state: Notified<AudioSnapshot>,
    pub clock_service_running: NotifiedAtomicBool,
    pub animation_state: ArcSwap<AnimationSnapshot>,
}

pub type SharedGuiState = Arc<GuiState>;

impl GuiState {
    pub fn new(repaint: RepaintSignal) -> Self {
        Self {
            midi_slots: Notified::new(Vec::new(), repaint.clone()),
            audio_state: Notified::new(AudioSnapshot::default(), repaint.clone()),
            clock_service_running: NotifiedAtomicBool::new(false, repaint),
            animation_state: ArcSwap::from_pointee(AnimationSnapshot::default()),
        }
    }
}
