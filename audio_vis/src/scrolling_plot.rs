//! Scrolling time-domain plot widget for egui_plot.
//! Supports multiple overlaid traces with a text legend.

use std::collections::VecDeque;

use eframe::egui::{self, Color32};
use egui_plot::{Line, Plot, PlotPoint, PlotPoints};

/// A single time-series trace (ring of timestamped samples).
pub struct Trace {
    pub points: VecDeque<[f64; 2]>,
    pub label: String,
    pub color: Color32,
}

impl Trace {
    pub fn new(label: impl Into<String>, color: Color32) -> Self {
        Self {
            points: VecDeque::new(),
            label: label.into(),
            color,
        }
    }

    /// Append new samples, evenly spaced backwards from `current_time`.
    pub fn ingest(&mut self, samples: &[f32], sample_interval: f64, current_time: f64) {
        if samples.is_empty() {
            return;
        }
        let first_time = current_time - (samples.len() - 1) as f64 * sample_interval;
        for (i, &value) in samples.iter().enumerate() {
            let t = first_time + i as f64 * sample_interval;
            self.points.push_back([t, value as f64]);
        }
    }

    /// Remove samples older than `cutoff`.
    fn trim(&mut self, cutoff: f64) {
        while let Some(front) = self.points.front() {
            if front[0] < cutoff {
                self.points.pop_front();
            } else {
                break;
            }
        }
    }

    fn plot_points(&self) -> Vec<PlotPoint> {
        self.points
            .iter()
            .map(|&[x, y]| PlotPoint::new(x, y))
            .collect()
    }
}

/// A scrolling time-domain plot that displays multiple overlaid traces.
pub struct ScrollingPlot {
    pub traces: Vec<Trace>,
    pub window_seconds: f64,
    y_min: f64,
    y_max: f64,
}

impl ScrollingPlot {
    pub fn new(window_seconds: f64, y_min: f64, y_max: f64) -> Self {
        Self {
            traces: Vec::new(),
            window_seconds,
            y_min,
            y_max,
        }
    }

    pub fn add_trace(&mut self, label: impl Into<String>, color: Color32) -> usize {
        let idx = self.traces.len();
        self.traces.push(Trace::new(label, color));
        idx
    }

    /// Trim all traces to the current time window.
    pub fn trim(&mut self, current_time: f64) {
        let cutoff = current_time - self.window_seconds;
        for trace in &mut self.traces {
            trace.trim(cutoff);
        }
    }

    /// Render the plot with enabled traces overlaid.
    /// `enabled` slice must be the same length as `self.traces`.
    /// `scales` optionally provides per-trace Y scaling (e.g. for normalization).
    /// Pass `None` for no scaling.
    pub fn ui(
        &self,
        ui: &mut egui::Ui,
        plot_id: &str,
        enabled: &[bool],
        scales: Option<&[f32]>,
    ) {
        let current_time = self
            .traces
            .iter()
            .enumerate()
            .filter(|(i, _)| enabled.get(*i).copied().unwrap_or(true))
            .filter_map(|(_, t)| t.points.back().map(|p| p[0]))
            .fold(0.0_f64, f64::max);

        let height = ui.available_height().max(100.0);

        Plot::new(plot_id)
            .default_x_bounds(current_time - self.window_seconds, current_time)
            .default_y_bounds(self.y_min, self.y_max)
            .height(height)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show_axes([false, true])
            .legend(egui_plot::Legend::default())
            .show(ui, |plot_ui| {
                for (i, trace) in self.traces.iter().enumerate() {
                    if enabled.get(i).copied().unwrap_or(true) {
                        let scale = scales
                            .and_then(|s| s.get(i).copied())
                            .unwrap_or(1.0) as f64;
                        let points = if scale != 1.0 {
                            trace
                                .points
                                .iter()
                                .map(|&[x, y]| PlotPoint::new(x, y * scale))
                                .collect()
                        } else {
                            trace.plot_points()
                        };
                        plot_ui.line(
                            Line::new(&trace.label, PlotPoints::Owned(points))
                                .color(trace.color)
                                .width(1.5),
                        );
                    }
                }
            });
    }
}
