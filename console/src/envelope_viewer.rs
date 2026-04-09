use eframe::egui::{self, Color32};
use gui_common::scrolling_plot::ScrollingPlot;
use tunnels::audio::processor::{NUM_OUTPUT_BANDS, SharedEnvelopeHistory};

const BAND_LABELS: [&str; NUM_OUTPUT_BANDS] = [
    "<187",
    "187-375",
    "375-750",
    "750-1.5k",
    "1.5-3k",
    "3-6k",
    "6-12k",
    "12-24k",
];

const BAND_COLORS: [Color32; NUM_OUTPUT_BANDS] = [
    Color32::from_rgb(160, 160, 160), // sub-bass — grey
    Color32::from_rgb(180, 80, 255),  // 187-375 — violet
    Color32::from_rgb(80, 120, 255),  // 375-750 — blue
    Color32::from_rgb(60, 220, 255),  // 750-1.5k — cyan
    Color32::from_rgb(60, 255, 120),  // 1.5-3k — green
    Color32::from_rgb(255, 240, 60),  // 3-6k — yellow
    Color32::from_rgb(255, 160, 40),  // 6-12k — orange
    Color32::from_rgb(255, 60, 60),   // 12-24k — red
];

/// Approximate update rate (~1kHz buffer callbacks).
const SAMPLE_RATE: f64 = 1000.0;

pub struct EnvelopeViewerState {
    open: bool,
    plot: ScrollingPlot,
    read_positions: [usize; NUM_OUTPUT_BANDS],
    initialized: bool,
    start_time: std::time::Instant,
}

impl EnvelopeViewerState {
    pub fn new() -> Self {
        let mut plot = ScrollingPlot::new(3.0, 0.0, 1.1);
        for i in 0..NUM_OUTPUT_BANDS {
            plot.add_trace(BAND_LABELS[i], BAND_COLORS[i]);
        }
        Self {
            open: false,
            plot,
            read_positions: [0; NUM_OUTPUT_BANDS],
            initialized: false,
            start_time: std::time::Instant::now(),
        }
    }

    /// Render the envelope viewer. Returns whether it's open (for layout purposes).
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        envelope_history: Option<&SharedEnvelopeHistory>,
    ) -> bool {
        let was_open = self.open;
        ui.checkbox(&mut self.open, "Envelope Viewer");

        let Some(history) = envelope_history else {
            if self.open {
                ui.label("No audio input connected.");
            }
            return self.open;
        };

        // Handle open/close transitions.
        if self.open && !was_open {
            // Just opened: enable streaming, initialize read positions.
            history
                .send_enabled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            for i in 0..NUM_OUTPUT_BANDS {
                self.read_positions[i] = history.histories[i].write_pos();
            }
            self.initialized = true;
        } else if !self.open && was_open {
            // Just closed: disable streaming, drain buffers, clear plot.
            history
                .send_enabled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            for i in 0..NUM_OUTPUT_BANDS {
                self.read_positions[i] = history.histories[i].write_pos();
            }
            for trace in &mut self.plot.traces {
                trace.points.clear();
            }
            self.initialized = false;
        }

        if !self.open {
            return false;
        }

        // Drain and display.
        let now = self.start_time.elapsed().as_secs_f64();
        let interval = 1.0 / SAMPLE_RATE;
        let mut samples = Vec::new();

        let all_enabled = [true; NUM_OUTPUT_BANDS];
        for i in 0..NUM_OUTPUT_BANDS {
            samples.clear();
            history.histories[i].drain_into(&mut samples, &mut self.read_positions[i]);
            self.plot.traces[i].ingest(&samples, interval, now);
        }
        self.plot.trim(now);

        // Render the plot.
        let height = 200.0_f32;
        self.plot.ui_with_options(
            ui,
            "envelope_viewer",
            &all_enabled,
            None, // no per-trace scaling
            None, // no link group
            Some(height),
            None, // no custom Y axis
        );

        true
    }
}
