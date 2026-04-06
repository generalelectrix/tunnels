pub mod scrolling_plot;

use std::time::Instant;

use eframe::egui::{self, Color32};

use tunnels::audio::log_scale::DEFAULT_RANGE_DB;
use tunnels::audio::processor::{ChainIdx, ProcessorSettings};

use scrolling_plot::ScrollingPlot;

/// Approximate update rate of the envelope (once per audio buffer, ~1kHz).
const ENVELOPE_SAMPLE_RATE: f64 = 1000.0;

// 1 input peak + 8 chains = 9 traces total.
const NUM_TRACES: usize = 9;

struct TraceConfig {
    label: &'static str,
    color: Color32,
    default_enabled: bool,
}

const TRACE_CONFIGS: [TraceConfig; NUM_TRACES] = [
    // Index 0: input peak
    TraceConfig {
        label: "Input peak",
        color: Color32::from_rgb(80, 80, 100),
        default_enabled: true,
    },
    // Indices 1-4: abs() rectifier × 4 reducers
    TraceConfig {
        label: "abs + env",
        color: Color32::from_rgb(70, 180, 130),
        default_enabled: true,
    },
    TraceConfig {
        label: "abs + two-stage",
        color: Color32::from_rgb(220, 200, 60),
        default_enabled: false,
    },
    TraceConfig {
        label: "abs + RMS",
        color: Color32::from_rgb(180, 180, 180),
        default_enabled: false,
    },
    TraceConfig {
        label: "abs + median",
        color: Color32::from_rgb(255, 140, 180),
        default_enabled: false,
    },
    // Indices 5-8: Hilbert rectifier × 4 reducers
    TraceConfig {
        label: "Hilbert + env",
        color: Color32::from_rgb(80, 200, 220),
        default_enabled: false,
    },
    TraceConfig {
        label: "Hilbert + two-stage",
        color: Color32::from_rgb(200, 100, 220),
        default_enabled: false,
    },
    TraceConfig {
        label: "Hilbert + RMS",
        color: Color32::WHITE,
        default_enabled: false,
    },
    TraceConfig {
        label: "Hilbert + median",
        color: Color32::from_rgb(255, 200, 100),
        default_enabled: false,
    },
];

pub struct AudioVisApp {
    processor_settings: ProcessorSettings,
    plot: ScrollingPlot,
    read_positions: [usize; NUM_TRACES],
    trace_enabled: [bool; NUM_TRACES],
    start_time: Instant,
    log_range_db: f32,
    log_enabled: bool,
    normalize_enabled: bool,
    running_max: [f32; NUM_TRACES],
    paused: bool,
}

impl AudioVisApp {
    pub fn new(processor_settings: ProcessorSettings) -> Self {
        let mut plot = ScrollingPlot::new(3.0, 0.0, 1.1);
        let mut trace_enabled = [false; NUM_TRACES];

        for (i, config) in TRACE_CONFIGS.iter().enumerate() {
            plot.add_trace(config.label, config.color);
            trace_enabled[i] = config.default_enabled;
        }

        // Index 0 = input_peak, indices 1-8 = chain_history[0-7]
        let s = &processor_settings;
        let read_positions = [
            s.input_peak_history.write_pos(),
            s.chain_history[ChainIdx::ABS_ENV].write_pos(),
            s.chain_history[ChainIdx::ABS_TWO_STAGE].write_pos(),
            s.chain_history[ChainIdx::ABS_RMS].write_pos(),
            s.chain_history[ChainIdx::ABS_MEDIAN].write_pos(),
            s.chain_history[ChainIdx::HILBERT_ENV].write_pos(),
            s.chain_history[ChainIdx::HILBERT_TWO_STAGE].write_pos(),
            s.chain_history[ChainIdx::HILBERT_RMS].write_pos(),
            s.chain_history[ChainIdx::HILBERT_MEDIAN].write_pos(),
        ];

        Self {
            processor_settings,
            plot,
            read_positions,
            trace_enabled,
            start_time: Instant::now(),
            log_range_db: DEFAULT_RANGE_DB,
            log_enabled: false,
            normalize_enabled: false,
            running_max: [0.001; NUM_TRACES],
            paused: false,
        }
    }

    fn current_time(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    fn drain_trace(
        buf: &tunnels::audio::ring_buffer::SignalRingBuffer,
        read_pos: &mut usize,
    ) -> Vec<f32> {
        let mut samples = Vec::new();
        buf.drain_into(&mut samples, read_pos);
        samples
    }

}

impl eframe::App for AudioVisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = self.current_time();
        let interval = 1.0 / ENVELOPE_SAMPLE_RATE;

        let log_enabled = self.log_enabled;
        let log_range = self.log_range_db;

        for i in 0..NUM_TRACES {
            let buf = if i == 0 {
                &self.processor_settings.input_peak_history
            } else {
                &self.processor_settings.chain_history[i - 1]
            };

            // Always drain to avoid ring buffer overflow.
            let mut samples = Self::drain_trace(buf, &mut self.read_positions[i]);

            if self.paused {
                continue;
            }

            if !self.trace_enabled[i] {
                self.plot.traces[i].points.clear();
                self.running_max[i] = 0.001;
                continue;
            }

            if log_enabled {
                for v in &mut samples {
                    *v = tunnels::audio::log_scale::linear_to_perceptual(*v, log_range);
                }
            }

            for &v in &samples {
                if v > self.running_max[i] {
                    self.running_max[i] = v;
                }
            }

            self.plot.traces[i].ingest(&samples, interval, now);
        }

        if !self.paused {
            self.plot.trim(now);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let ps = &self.processor_settings;

            ui.columns(2, |cols| {
                // Left column: signal + envelope parameters.
                cols[0].strong("Signal");
                egui::Grid::new("signal_grid").show(&mut cols[0], |ui| {
                    ui.label("Input gain:");
                    let mut gain_db = 20.0 * ps.gain.get().log10();
                    if ui.add(egui::Slider::new(&mut gain_db, -20.0..=30.0).suffix(" dB")).changed() {
                        ps.gain.set(10.0_f32.powf(gain_db / 20.0));
                    }
                    ui.end_row();

                    ui.label("Filter cutoff:");
                    let mut v = ps.filter_cutoff.get();
                    if ui.add(egui::Slider::new(&mut v, 40.0..=1040.0).suffix(" Hz").logarithmic(true)).changed() {
                        ps.filter_cutoff.set(v);
                    }
                    ui.end_row();

                    ui.label("Env attack:");
                    let mut v = ps.envelope_attack.get() * 1000.0;
                    if ui.add(egui::Slider::new(&mut v, 1.0..=256.0).suffix(" ms").logarithmic(true)).changed() {
                        ps.envelope_attack.set(v / 1000.0);
                    }
                    ui.end_row();

                    ui.label("Env release:");
                    let mut v = ps.envelope_release.get() * 1000.0;
                    if ui.add(egui::Slider::new(&mut v, 1.0..=1000.0).suffix(" ms").logarithmic(true)).changed() {
                        ps.envelope_release.set(v / 1000.0);
                    }
                    ui.end_row();
                });

                cols[0].separator();
                cols[0].strong("Two-Stage");
                egui::Grid::new("two_stage_grid").show(&mut cols[0], |ui| {
                    ui.label("Fast attack:");
                    let mut v = ps.fast_attack.get() * 1000.0;
                    if ui.add(egui::Slider::new(&mut v, 0.1..=50.0).suffix(" ms").logarithmic(true)).changed() {
                        ps.fast_attack.set(v / 1000.0);
                    }
                    ui.end_row();

                    ui.label("Fast release:");
                    let mut v = ps.fast_release.get() * 1000.0;
                    if ui.add(egui::Slider::new(&mut v, 1.0..=200.0).suffix(" ms").logarithmic(true)).changed() {
                        ps.fast_release.set(v / 1000.0);
                    }
                    ui.end_row();
                });

                cols[0].separator();
                cols[0].strong("RMS / Median");
                egui::Grid::new("reducer_grid").show(&mut cols[0], |ui| {
                    ui.label("Window:");
                    let mut v = ps.reducer_window.get() * 1000.0;
                    if ui.add(egui::Slider::new(&mut v, 1.0..=200.0).suffix(" ms").logarithmic(true)).changed() {
                        ps.reducer_window.set(v / 1000.0);
                    }
                    ui.end_row();
                });

                // Right column: display controls.
                cols[1].strong("Display");
                if cols[1].button(if self.paused { "▶ Resume" } else { "⏸ Pause" }).clicked() {
                    self.paused = !self.paused;
                }
                egui::Grid::new("display_grid").show(&mut cols[1], |ui| {
                    ui.checkbox(&mut self.log_enabled, "Log transform");
                    if self.log_enabled {
                        ui.add(egui::Slider::new(&mut self.log_range_db, 6.0..=60.0).suffix(" dB"));
                    }
                    ui.end_row();

                    ui.checkbox(&mut self.normalize_enabled, "Normalize traces");
                    ui.end_row();

                    ui.label("Window:");
                    let mut secs = self.plot.window_seconds as f32;
                    if ui.add(egui::Slider::new(&mut secs, 0.5..=15.0).suffix(" s").logarithmic(true)).changed() {
                        self.plot.window_seconds = secs as f64;
                    }
                    ui.end_row();
                });
            });

            ui.separator();

            // Trace toggles.
            ui.horizontal_wrapped(|ui| {
                for (i, config) in TRACE_CONFIGS.iter().enumerate() {
                    let mut enabled = self.trace_enabled[i];
                    let label = egui::RichText::new(config.label).color(if enabled {
                        config.color
                    } else {
                        Color32::GRAY
                    });
                    if ui.checkbox(&mut enabled, label).changed() {
                        self.trace_enabled[i] = enabled;
                    }
                }
            });

            ui.separator();

            // Plot.
            let scales: Option<Vec<f32>> = if self.normalize_enabled {
                Some(self.running_max.iter().map(|m| 1.0 / m).collect())
            } else {
                None
            };
            self.plot.ui(
                ui,
                "audio_signals",
                &self.trace_enabled,
                scales.as_deref(),
            );
        });

        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use tunnels::audio::processor::{Processor, ProcessorSettings};

    fn make_app() -> AudioVisApp {
        AudioVisApp::new(ProcessorSettings::default())
    }

    fn process_signal_and_plot(
        name: &str,
        duration_secs: f32,
        signal_fn: impl Fn(usize, f32) -> f32,
    ) {
        let sample_rate = 48000_u32;
        let channels = 1_usize;
        let buffer_size = 48;
        let total_samples = (duration_secs * sample_rate as f32) as usize;

        let settings = ProcessorSettings::default();
        let mut processor = Processor::new(settings.clone(), sample_rate, channels);

        let mut sample_idx = 0;
        while sample_idx < total_samples {
            let chunk_end = (sample_idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (sample_idx..chunk_end)
                .map(|i| signal_fn(i, sample_rate as f32))
                .collect();
            processor.process(&buffer);
            sample_idx = chunk_end;
        }

        let mut plot = ScrollingPlot::new(duration_secs as f64, 0.0, 1.1);
        for config in &TRACE_CONFIGS {
            plot.add_trace(config.label, config.color);
        }

        let update_interval = 1.0 / 1000.0;
        let enabled = [true; NUM_TRACES];

        // Drain input peak (trace 0)
        {
            let mut pos = 0_usize;
            let mut samples = Vec::new();
            settings
                .input_peak_history
                .drain_into(&mut samples, &mut pos);
            let t = samples.len() as f64 * update_interval;
            plot.traces[0].ingest(&samples, update_interval, t);
        }

        // Drain chain histories (traces 1-8)
        for chain_idx in 0..8 {
            let mut pos = 0_usize;
            let mut samples = Vec::new();
            settings.chain_history[chain_idx].drain_into(&mut samples, &mut pos);
            let t = samples.len() as f64 * update_interval;
            plot.traces[chain_idx + 1].ingest(&samples, update_interval, t);
        }

        let mut harness = Harness::new_ui(|ui| {
            ui.heading(name);
            ui.separator();
            plot.ui(ui, name, &enabled, None);
        });
        harness.run();
        harness.snapshot(name);
    }

    #[test]
    fn render_empty() {
        let mut app = make_app();
        let mut harness = Harness::new_ui(|ui| {
            ui.heading("Audio Visualizer");
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for (i, config) in TRACE_CONFIGS.iter().enumerate() {
                    let label = egui::RichText::new(config.label).color(config.color);
                    ui.checkbox(&mut app.trace_enabled[i], label);
                }
            });
            ui.separator();
            app.plot.ui(ui, "audio_signals", &app.trace_enabled, None);
        });
        harness.run();
        harness.snapshot("audio_vis_empty");
    }

    #[test]
    fn signal_sine_burst() {
        process_signal_and_plot("sine_burst", 1.0, |i, sr| {
            let t = i as f32 / sr;
            if (0.2..0.5).contains(&t) {
                (2.0 * std::f32::consts::PI * 100.0 * t).sin() * 0.8
            } else {
                0.0
            }
        });
    }

    #[test]
    fn signal_impulse() {
        process_signal_and_plot("impulse", 1.0, |i, sr| {
            let t = i as f32 / sr;
            if (t - 0.3).abs() < 1.0 / sr {
                1.0
            } else {
                0.0
            }
        });
    }

    #[test]
    fn signal_repeated_kicks() {
        process_signal_and_plot("repeated_kicks", 2.0, |i, sr| {
            let t = i as f32 / sr;
            let period = 0.5;
            let phase_in_period = t % period;
            if phase_in_period < 0.010 {
                let decay = (-phase_in_period * 200.0).exp();
                (2.0 * std::f32::consts::PI * 60.0 * t).sin() * 0.9 * decay
            } else {
                0.0
            }
        });
    }

    #[test]
    fn signal_quiet_to_loud() {
        process_signal_and_plot("quiet_to_loud", 1.5, |i, sr| {
            let t = i as f32 / sr;
            let amplitude = if t < 0.5 { 0.1 } else { 0.8 };
            (2.0 * std::f32::consts::PI * 200.0 * t).sin() * amplitude
        });
    }
}
