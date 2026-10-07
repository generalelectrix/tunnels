use eframe::egui::{self, Color32};
use tunnels_audio::UnipolarF32;
use tunnels_audio::processor::UpdateRate;
use tunnels_audio::roles::{NUM_ROLES, Role};
use tunnels_audio::{EnvelopeStream, EnvelopeStreams};

use crate::scrolling_plot::ScrollingPlot;

/// One colour per role, in [`Role::ALL`] order.
const ROLE_COLORS: [Color32; NUM_ROLES] = [
    Color32::from_rgb(255, 155, 61),  // Kick — orange
    Color32::from_rgb(167, 149, 255), // Bass — violet
    Color32::from_rgb(95, 208, 214),  // Mid — cyan
    Color32::from_rgb(217, 227, 106), // Hats — yellow-green
    Color32::from_rgb(240, 163, 230), // Shimmer — pink
];

pub struct EnvelopeViewerState {
    open: bool,
    plot: ScrollingPlot,
    envelope_streams: Option<[EnvelopeStream; NUM_ROLES]>,
    update_rate: Option<UpdateRate>,
    start_time: std::time::Instant,
}

impl Default for EnvelopeViewerState {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvelopeViewerState {
    pub fn new() -> Self {
        let mut plot = ScrollingPlot::new(3.0, 0.0, 1.1);
        for (role, color) in Role::ALL.into_iter().zip(ROLE_COLORS) {
            plot.add_trace(role.label(), color);
        }
        Self {
            open: false,
            plot,
            envelope_streams: None,
            update_rate: None,
            start_time: std::time::Instant::now(),
        }
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            // Clear the plot when closing.
            for trace in &mut self.plot.traces {
                trace.points.clear();
            }
        }
    }

    /// Provide new envelope streams (e.g. after a device change).
    pub fn set_envelope_streams(&mut self, new_streams: EnvelopeStreams) {
        // The new device's history starts now.
        for trace in &mut self.plot.traces {
            trace.points.clear();
        }
        self.envelope_streams = Some(new_streams.streams);
        self.update_rate = Some(new_streams.update_rate);
    }

    /// Render the envelope viewer. Returns whether it's open (for layout purposes).
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let was_open = self.open;
        ui.checkbox(&mut self.open, "Envelope Viewer");

        let Some(envelope_streams) = &mut self.envelope_streams else {
            if self.open {
                ui.label("No audio input connected.");
            }
            return self.open;
        };

        // Handle close transition: clear the plot and drain stale data.
        if !self.open && was_open {
            for stream in envelope_streams.iter_mut() {
                stream.clear();
            }
            for trace in &mut self.plot.traces {
                trace.points.clear();
            }
        }

        if !self.open {
            return false;
        }

        // Handle open transition: drain any stale data from before we started watching.
        if self.open && !was_open {
            for stream in envelope_streams.iter_mut() {
                stream.clear();
            }
        }

        // Drain and display.
        let now = self.start_time.elapsed().as_secs_f64();
        let Some(rate) = self.update_rate else {
            ui.label("Waiting for audio data...");
            return true;
        };
        let interval = rate.interval_secs();
        let mut drained: Vec<UnipolarF32> = Vec::new();
        let mut samples: Vec<f32> = Vec::new();

        for (i, stream) in envelope_streams.iter_mut().enumerate() {
            drained.clear();
            stream.drain_into(&mut drained);
            samples.clear();
            samples.extend(drained.iter().map(|&v| f32::from(v)));
            self.plot.traces[i].ingest(&samples, interval, now);
        }
        self.plot.trim(now);

        // Render the plot — fill remaining vertical space, minimum 150px.
        let height = ui.available_height().max(150.0);
        let all_enabled = [true; NUM_ROLES];
        self.plot.ui_with_options(
            ui,
            "envelope_viewer",
            &all_enabled,
            None,
            None,
            Some(height),
            None,
        );

        // Continuously repaint while the viewer is open.
        ui.ctx().request_repaint();

        true
    }
}
