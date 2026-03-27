use std::sync::{Arc, atomic::AtomicBool};

use arc_swap::ArcSwap;
use midi_harness::SlotStatus;

use crate::animation_visualizer::AnimationSnapshot;

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
