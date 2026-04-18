use std::sync::{Arc, Mutex, atomic::AtomicBool};
use std::time::Duration;

use arc_swap::ArcSwap;
use midi_harness::SlotStatus;
use tunnels_audio::EnvelopeStreams;
use tunnels_audio::processor::TrackingMode;
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

/// Snapshot of audio subsystem state for the GUI.
#[derive(Debug, Clone)]
pub struct AudioStateSnapshot {
    pub device_name: String,
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
}

impl Default for AudioStateSnapshot {
    fn default() -> Self {
        Self {
            device_name: tunnels_audio::OFFLINE_DEVICE_NAME.to_string(),
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
        }
    }
}

pub struct GuiState {
    /// Show → GUI: MIDI slot statuses. Wakes the GUI on write.
    pub midi_slots: Notified<Vec<SlotStatus>>,
    /// Show → GUI: audio subsystem snapshot. Wakes the GUI on write.
    pub audio_state: Notified<AudioStateSnapshot>,
    /// Show → GUI: whether the clock publisher service is running. Wakes the GUI on write.
    pub clock_service_running: NotifiedAtomicBool,
    /// Show → GUI: animation state streamed at render rate while the visualizer
    /// is active. The visualizer panel drives its own repaint — wrapping in
    /// `Notified` here would just collapse to continuous repaint.
    pub animation_state: ArcSwap<AnimationSnapshot>,
    /// GUI → Show: whether the animation visualizer is visible. Read by the
    /// show to decide whether to snapshot animation state. No repaint needed
    /// (the GUI is the writer).
    pub visualizer_active: AtomicBool,
    /// Envelope ring buffer streams and update rate for the GUI viewer.
    /// Placed by the show thread on device change, taken by the GUI thread.
    /// The envelope viewer drives its own continuous repaint while open.
    pub envelope_streams: Mutex<Option<EnvelopeStreams>>,
}

pub type SharedGuiState = Arc<GuiState>;

impl GuiState {
    pub fn new(repaint: RepaintSignal) -> Self {
        Self {
            midi_slots: Notified::new(Vec::new(), repaint.clone()),
            audio_state: Notified::new(AudioStateSnapshot::default(), repaint.clone()),
            clock_service_running: NotifiedAtomicBool::new(false, repaint),
            animation_state: ArcSwap::from_pointee(AnimationSnapshot::default()),
            visualizer_active: AtomicBool::new(false),
            envelope_streams: Mutex::new(None),
        }
    }
}
