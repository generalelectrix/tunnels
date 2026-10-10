use std::sync::Arc;

use arc_swap::ArcSwap;
use midi_harness::SlotStatus;
use tunnels_audio::{AudioSnapshot, Role};
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
        const TOUCHOSC     = 0b0000_1000;
    }
}

/// Shared state readable by the GUI. A store to a `Notified` field wakes the
/// GUI; `animation_state` is a raw `ArcSwap`, whose stores do not.
pub struct GuiState {
    pub midi_slots: Notified<Vec<SlotStatus>>,
    pub audio_state: Notified<AudioSnapshot>,
    /// The role the show follows.
    pub active_role: Notified<Role>,
    /// Whether the live audio input's clip indicator is lit, or `None` with no
    /// live input.
    pub input_clip_lit: Notified<Option<bool>>,
    /// The live audio input's trim in dB, to the nearest
    /// `TRIM_DISPLAY_STEP_DB`, or `None` with no live input.
    pub input_trim_db: Notified<Option<f32>>,
    pub clock_service_running: NotifiedAtomicBool,
    pub touchosc_server_running: NotifiedAtomicBool,
    pub animation_state: ArcSwap<AnimationSnapshot>,
}

pub type SharedGuiState = Arc<GuiState>;

impl GuiState {
    pub fn new(repaint: RepaintSignal) -> Self {
        Self {
            midi_slots: Notified::new(Vec::new(), repaint.clone()),
            audio_state: Notified::new(AudioSnapshot::default(), repaint.clone()),
            active_role: Notified::new(Role::default(), repaint.clone()),
            input_clip_lit: Notified::new(None, repaint.clone()),
            input_trim_db: Notified::new(None, repaint.clone()),
            clock_service_running: NotifiedAtomicBool::new(false, repaint.clone()),
            touchosc_server_running: NotifiedAtomicBool::new(false, repaint),
            animation_state: ArcSwap::from_pointee(AnimationSnapshot::default()),
        }
    }
}
