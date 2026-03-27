use std::sync::{atomic::AtomicBool, Arc};

use arc_swap::ArcSwap;
use midi_harness::SlotStatus;

use crate::animation_visualizer::AnimationSnapshot;

bitflags::bitflags! {
    /// GUI state domains that may need re-snapshotting after a control event.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GuiDirty: u8 {
        const CLEAN        = 0b0000_0000;
        const MIDI_SLOTS   = 0b0000_0001;
        const AUDIO_DEVICE = 0b0000_0010;
    }
}

pub struct GuiState {
    pub midi_slots: ArcSwap<Vec<SlotStatus>>,
    pub audio_device: ArcSwap<String>,
    pub visualizer_active: AtomicBool,
    pub animation_state: ArcSwap<AnimationSnapshot>,
}

pub type SharedGuiState = Arc<GuiState>;

impl GuiState {
    pub fn new() -> Self {
        Self {
            midi_slots: ArcSwap::from_pointee(Vec::new()),
            audio_device: ArcSwap::from_pointee("Offline".to_string()),
            visualizer_active: AtomicBool::new(false),
            animation_state: ArcSwap::from_pointee(AnimationSnapshot::default()),
        }
    }
}
