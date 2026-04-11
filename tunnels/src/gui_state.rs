use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

use arc_swap::ArcSwap;
use midi_harness::SlotStatus;
use tunnels_audio::processor::{SharedEnvelopeHistory, TrackingMode, UpdateRate};

use crate::animation_visualizer::AnimationSnapshot;

bitflags::bitflags! {
    /// GUI state domains that may need re-snapshotting after a control event.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GuiDirty: u8 {
        const CLEAN         = 0b0000_0000;
        const MIDI_SLOTS    = 0b0000_0001;
        const AUDIO_DEVICE  = 0b0000_0010;
        const CLOCK_SERVICE = 0b0000_0100;
        const AUDIO_STATE   = 0b0000_1000;
    }
}

/// Snapshot of audio subsystem state for the GUI.
#[derive(Debug, Clone)]
/// Snapshot of audio parameter state for the GUI.
/// Contains only values that change on user action (not streaming data).
pub struct AudioStateSnapshot {
    pub filter_cutoff_hz: f32,
    pub envelope_attack: Duration,
    pub envelope_release: Duration,
    pub output_smoothing: Duration,
    pub gain_linear: f64,
    pub auto_trim_enabled: bool,
    pub active_band: u32,
    pub norm_floor_halflife: Duration,
    pub norm_ceiling_halflife: Duration,
    pub norm_floor_mode: TrackingMode,
    pub norm_ceiling_mode: TrackingMode,
    /// Audio callback rate (sample_rate / frames_per_buffer).
    pub update_rate: Option<UpdateRate>,
}

impl Default for AudioStateSnapshot {
    fn default() -> Self {
        Self {
            filter_cutoff_hz: 200.0,
            envelope_attack: Duration::from_millis(10),
            envelope_release: Duration::from_millis(50),
            output_smoothing: Duration::from_millis(8),
            gain_linear: 1.0,
            auto_trim_enabled: true,
            active_band: 0,
            norm_floor_halflife: Duration::from_secs(10),
            norm_ceiling_halflife: Duration::from_secs(5),
            norm_floor_mode: TrackingMode::Average,
            norm_ceiling_mode: TrackingMode::Limit,
            update_rate: None,
        }
    }
}

pub struct GuiState {
    pub midi_slots: ArcSwap<Vec<SlotStatus>>,
    pub audio_device: ArcSwap<String>,
    pub audio_state: ArcSwap<AudioStateSnapshot>,
    pub clock_service_running: AtomicBool,
    pub visualizer_active: AtomicBool,
    pub animation_state: ArcSwap<AnimationSnapshot>,
    /// Shared envelope ring buffers for the envelope viewer panel.
    /// Set by the show thread once `AudioInput` is initialized.
    pub envelope_history: ArcSwap<Option<SharedEnvelopeHistory>>,
}

pub type SharedGuiState = Arc<GuiState>;

impl Default for GuiState {
    fn default() -> Self {
        Self {
            midi_slots: ArcSwap::from_pointee(Vec::new()),
            audio_device: ArcSwap::from_pointee("Offline".to_string()),
            audio_state: ArcSwap::from_pointee(AudioStateSnapshot::default()),
            clock_service_running: AtomicBool::new(false),
            visualizer_active: AtomicBool::new(false),
            animation_state: ArcSwap::from_pointee(AnimationSnapshot::default()),
            envelope_history: ArcSwap::from_pointee(None),
        }
    }
}
