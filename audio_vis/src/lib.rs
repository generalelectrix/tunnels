pub mod scrolling_plot;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};

use gui_common::audio_panel::{AudioCommands, AudioPanel, AudioPanelState, AudioSnapshot};
use tunnels_audio::log_scale::DEFAULT_RANGE_DB;
use tunnels_audio::processor::ProcessorSettings;
use tunnels_audio::spectral::{self, SharedSpectralSnapshot};

use scrolling_plot::ScrollingPlot;

/// Approximate update rate of the envelope (once per audio buffer, ~1kHz).
const ENVELOPE_SAMPLE_RATE: f64 = 1000.0;

/// Number of signal traces (main plot).
const NUM_SIGNAL_TRACES: usize = 3;

struct TraceConfig {
    label: &'static str,
    color: Color32,
    default_enabled: bool,
}

const SIGNAL_TRACE_CONFIGS: [TraceConfig; NUM_SIGNAL_TRACES] = [
    TraceConfig {
        label: "Input peak",
        color: Color32::from_rgb(80, 80, 100),
        default_enabled: false,
    },
    TraceConfig {
        label: "Envelope",
        color: Color32::from_rgb(70, 180, 130),
        default_enabled: false,
    },
    TraceConfig {
        label: "Smoothed",
        color: Color32::WHITE,
        default_enabled: false,
    },
];

const GAIN_TRACE_COLOR: Color32 = Color32::from_rgb(200, 160, 60);
const TARGET_GAIN_TRACE_COLOR: Color32 = Color32::from_rgb(120, 120, 180);

/// Adapter that implements AudioCommands by writing directly to ProcessorSettings
/// atomics. Used by audio_vis because there's no show loop to mediate.
struct DirectAudioCommands<'a> {
    settings: &'a ProcessorSettings,
    pending_device: Option<Option<String>>,
}

impl AudioCommands for DirectAudioCommands<'_> {
    fn set_device(&mut self, device: Option<String>) {
        self.pending_device = Some(device);
    }
    fn set_filter_cutoff(&mut self, hz: f32) {
        self.settings.filter_cutoff.set(hz);
    }
    fn set_envelope_attack(&mut self, duration: Duration) {
        self.settings.envelope_attack.set(duration.as_secs_f32());
    }
    fn set_envelope_release(&mut self, duration: Duration) {
        self.settings.envelope_release.set(duration.as_secs_f32());
    }
    fn set_output_smoothing(&mut self, duration: Duration) {
        self.settings.output_smoothing.set(duration.as_secs_f32());
    }
    fn set_gain(&mut self, gain_linear: f64) {
        self.settings.gain.set(gain_linear as f32);
    }
    fn set_auto_trim_enabled(&mut self, enabled: bool) {
        self.settings
            .auto_trim_enabled
            .set(if enabled { 1.0 } else { 0.0 });
    }
    fn toggle_monitor(&mut self) {}
    fn reset_parameters(&mut self) {
        self.settings.reset_defaults();
    }
    fn list_devices(&mut self) -> Vec<String> {
        tunnels_audio::AudioInput::devices().unwrap_or_default()
    }
    fn report_error(&mut self, _error: impl std::fmt::Display) {}
}

pub struct AudioVisApp {
    processor_settings: ProcessorSettings,
    _input: Option<tunnels_audio::reconnect::ReconnectingInput>,
    _spectral_stop: Option<Box<dyn FnOnce() + Send>>,
    spectral_snapshot: SharedSpectralSnapshot,
    steering_params: std::sync::Arc<tunnels_audio::band_steering::SharedSteeringParams>,
    pending_device: Option<Option<String>>,
    audio_panel_state: AudioPanelState,
    /// Main signal plot (0-1 range): input peak, envelope, smoothed.
    signal_plot: ScrollingPlot,
    /// Auto-trim gain plot (dB range): separate Y axis.
    trim_plot: ScrollingPlot,
    signal_read_positions: [usize; NUM_SIGNAL_TRACES],
    gain_read_position: usize,
    target_gain_read_position: usize,
    signal_trace_enabled: [bool; NUM_SIGNAL_TRACES],
    trim_trace_enabled: bool,
    spectrum_visible: bool,
    spectrum_1f_weighted: bool,
    spectrum_log_freq: bool,
    /// Manual Y-axis max for spectrum plot.
    spectrum_y_max: f32,
    start_time: Instant,
    log_range_db: f32,
    log_enabled: bool,
    normalize_enabled: bool,
    running_max: [f32; NUM_SIGNAL_TRACES],
    paused: bool,
}

impl AudioVisApp {
    pub fn new(processor_settings: ProcessorSettings) -> Self {
        let mut signal_plot = ScrollingPlot::new(3.0, 0.0, 1.1);
        let mut signal_trace_enabled = [false; NUM_SIGNAL_TRACES];

        for (i, config) in SIGNAL_TRACE_CONFIGS.iter().enumerate() {
            signal_plot.add_trace(config.label, config.color);
            signal_trace_enabled[i] = config.default_enabled;
        }

        // Gain plot: Y range in linear gain. Auto-trim range is +-10 dB
        // (0.316x to 3.162x).
        let mut trim_plot = ScrollingPlot::new(3.0, 0.0, 3.5);
        trim_plot.add_trace("Effective gain", GAIN_TRACE_COLOR);
        trim_plot.add_trace("Target gain", TARGET_GAIN_TRACE_COLOR);

        let signal_read_positions = [
            processor_settings.input_peak_history.write_pos(),
            processor_settings.envelope_history.write_pos(),
            processor_settings.smoothed_history.write_pos(),
        ];
        let gain_read_position = processor_settings.effective_gain_history.write_pos();
        let target_gain_read_position = processor_settings.target_gain_history.write_pos();

        let devices = tunnels_audio::AudioInput::devices().unwrap_or_default();

        let spectral_snapshot = spectral::new_shared_snapshot();
        let steering_params = tunnels_audio::band_steering::SharedSteeringParams::new();
        let spectral_stop = spectral::start_spectral_thread(
            processor_settings.clone(),
            48000.0,
            spectral_snapshot.clone(),
            steering_params.clone(),
        );

        Self {
            processor_settings,
            _input: None,
            _spectral_stop: Some(spectral_stop),
            spectral_snapshot,
            steering_params,
            pending_device: None,
            audio_panel_state: AudioPanelState::new(devices),
            signal_plot,
            trim_plot,
            signal_read_positions,
            gain_read_position,
            target_gain_read_position,
            signal_trace_enabled,
            trim_trace_enabled: false,
            spectrum_visible: true,
            spectrum_1f_weighted: false,
            spectrum_log_freq: true,
            spectrum_y_max: 0.01,
            start_time: Instant::now(),
            log_range_db: DEFAULT_RANGE_DB,
            log_enabled: false,
            normalize_enabled: false,
            running_max: [0.001; NUM_SIGNAL_TRACES],
            paused: false,
        }
    }

    fn current_time(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Render the frequency-domain spectrum plot.
    fn render_spectrum_plot(
        ui: &mut egui::Ui,
        snap: &tunnels_audio::spectral::SpectralSnapshot,
        height: f32,
        weighted: bool,
        log_freq: bool,
        y_max: f32,
    ) {
        use egui_plot::{AxisHints, Line, Plot, PlotPoint, PlotPoints, VLine};

        if snap.frequencies.is_empty() {
            ui.label("Waiting for spectral data...");
            return;
        }

        let plot_y_max = y_max as f64;

        let freq_to_x = |f: f32| -> f64 {
            if log_freq {
                (f.max(1.0) as f64).log10()
            } else {
                f as f64
            }
        };

        let min_freq = 20.0_f32;

        let hz_axis = if log_freq {
            AxisHints::new_x()
                .label("Frequency")
                .min_thickness(24.0)
                .formatter(|mark, _range| {
                    let f = 10.0_f64.powf(mark.value);
                    if f >= 1000.0 {
                        format!("{:.1}k", f / 1000.0)
                    } else {
                        format!("{:.0}", f)
                    }
                })
        } else {
            AxisHints::new_x()
                .label("Frequency")
                .min_thickness(24.0)
                .formatter(|mark, _range| {
                    let f = mark.value;
                    if f >= 1000.0 {
                        format!("{:.1}k", f / 1000.0)
                    } else {
                        format!("{:.0} Hz", f)
                    }
                })
        };

        Plot::new("spectrum_plot")
            .height(height)
            .include_y(0.0)
            .include_y(plot_y_max)
            .auto_bounds(egui::Vec2b::new(true, false))
            .reset()
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show_axes([true, true])
            .custom_x_axes(vec![hz_axis])
            .legend(egui_plot::Legend::default())
            .show(ui, |plot_ui| {
                // Interest*Quality: the mass distribution that drives band steering.
                let interest_accum_data = if weighted {
                    &snap.interest_accum_weighted
                } else {
                    &snap.interest_accum
                };
                let mag_avg_data = if weighted {
                    &snap.magnitude_avg_weighted
                } else {
                    &snap.magnitude_avg
                };
                let iq_points: Vec<PlotPoint> = snap
                    .frequencies
                    .iter()
                    .zip(interest_accum_data.iter().zip(mag_avg_data))
                    .filter(|&(&f, _)| f >= min_freq)
                    .map(|(&f, (&interest, &mag))| {
                        let iq = if mag > 1e-10 {
                            interest * interest / mag
                        } else {
                            0.0
                        };
                        PlotPoint::new(freq_to_x(f), iq as f64)
                    })
                    .collect();
                plot_ui.line(
                    Line::new("Interest*Quality", PlotPoints::Owned(iq_points))
                        .color(Color32::from_rgb(255, 100, 255))
                        .width(1.5),
                );

                // Score surface: the convolution of the filter response
                // with the IQ distribution. Peaks = optimal filter positions.
                let score_points: Vec<PlotPoint> = snap
                    .band_steering
                    .score_surface
                    .iter()
                    .filter(|(f, _)| *f >= min_freq)
                    .map(|&(f, s)| PlotPoint::new(freq_to_x(f), s as f64))
                    .collect();
                if !score_points.is_empty() {
                    plot_ui.line(
                        Line::new("Score", PlotPoints::Owned(score_points))
                            .color(Color32::from_rgb(100, 255, 100))
                            .width(1.5),
                    );
                }

                // Lane boundaries as vertical lines.
                let lane_boundary_color = Color32::from_rgba_premultiplied(255, 255, 255, 40);
                let mut seen_boundaries = std::collections::HashSet::new();
                for filter in &snap.band_steering.filters {
                    for &boundary in &[filter.lane_min_hz, filter.lane_max_hz] {
                        let key = (boundary * 10.0) as u32;
                        if seen_boundaries.insert(key) {
                            plot_ui.vline(
                                VLine::new(
                                    format!("{:.0} Hz boundary", boundary),
                                    freq_to_x(boundary),
                                )
                                .color(lane_boundary_color)
                                .width(1.0),
                            );
                        }
                    }
                }

                // Filter response curves from band steering.
                let lane_colors = [
                    Color32::from_rgb(255, 100, 100),  // red — sub-bass
                    Color32::from_rgb(100, 200, 255),  // blue — low-mid
                    Color32::from_rgb(255, 255, 100),  // yellow — mid
                    Color32::from_rgb(100, 255, 200),  // teal — high
                ];
                let sample_rate = snap.sample_rate;
                let q = snap.band_steering.q;
                for filter in &snap.band_steering.filters {
                    let color = lane_colors[filter.lane_index % lane_colors.len()];
                    // Sample the filter response across the frequency range.
                    let response_points: Vec<PlotPoint> = snap
                        .frequencies
                        .iter()
                        .filter(|&&f| f >= min_freq)
                        .map(|&f| {
                            let r = filter.response_at(f, sample_rate, q);
                            // Scale response to match the Y axis — multiply by
                            // the current Y max so the peak of the filter curve
                            // reaches the top of the plot.
                            PlotPoint::new(freq_to_x(f), (r * y_max) as f64)
                        })
                        .collect();
                    plot_ui.line(
                        Line::new(
                            format!("{:.0} Hz", filter.center_hz),
                            PlotPoints::Owned(response_points),
                        )
                        .color(color)
                        .width(1.5),
                    );
                }
            });
    }

    /// Show detected peaks in a bottom status bar.
    fn render_spectrum_info(&self, ctx: &egui::Context) {
        if !self.spectrum_visible {
            return;
        }
        let snap = self.spectral_snapshot.load();
        let peaks = if self.spectrum_1f_weighted {
            &snap.peaks_weighted
        } else {
            &snap.peaks
        };
        if peaks.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("spectral_info").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, peak) in peaks.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.label(format!(
                        "{:.0} Hz ({:.0} wide)",
                        peak.center_hz, peak.bandwidth_hz,
                    ));
                }
            });
        });
    }

    fn drain_trace(
        buf: &tunnels_audio::ring_buffer::SignalRingBuffer,
        read_pos: &mut usize,
    ) -> Vec<f32> {
        let mut samples = Vec::new();
        buf.drain_into(&mut samples, read_pos);
        samples
    }

    fn snapshot(&self) -> AudioSnapshot {
        let ps = &self.processor_settings;
        let device_name = self
            ._input
            .as_ref()
            .map_or("Offline".to_string(), |_| "Connected".to_string());
        AudioSnapshot {
            device_name,
            filter_cutoff_hz: ps.filter_cutoff.get(),
            envelope_attack: Duration::from_secs_f32(ps.envelope_attack.get()),
            envelope_release: Duration::from_secs_f32(ps.envelope_release.get()),
            output_smoothing: Duration::from_secs_f32(ps.output_smoothing.get()),
            gain_linear: ps.gain.get() as f64,
            auto_trim_enabled: ps.auto_trim_enabled.get() > 0.5,
        }
    }
}

impl eframe::App for AudioVisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = self.current_time();
        let interval = 1.0 / ENVELOPE_SAMPLE_RATE;

        let log_enabled = self.log_enabled;
        let log_range = self.log_range_db;

        // Drain signal traces.
        let signal_buffers: [&tunnels_audio::ring_buffer::SignalRingBuffer; NUM_SIGNAL_TRACES] = [
            &self.processor_settings.input_peak_history,
            &self.processor_settings.envelope_history,
            &self.processor_settings.smoothed_history,
        ];

        for (i, buf) in signal_buffers.iter().enumerate() {
            let mut samples = Self::drain_trace(buf, &mut self.signal_read_positions[i]);

            if self.paused {
                continue;
            }

            if !self.signal_trace_enabled[i] {
                self.signal_plot.traces[i].points.clear();
                self.running_max[i] = 0.001;
                continue;
            }

            if log_enabled {
                for v in &mut samples {
                    *v = tunnels_audio::log_scale::linear_to_perceptual(*v, log_range);
                }
            }

            for &v in &samples {
                if v > self.running_max[i] {
                    self.running_max[i] = v;
                }
            }

            self.signal_plot.traces[i].ingest(&samples, interval, now);
        }

        // Drain gain traces (effective + target).
        {
            let gain_samples = Self::drain_trace(
                &self.processor_settings.effective_gain_history,
                &mut self.gain_read_position,
            );
            let target_samples = Self::drain_trace(
                &self.processor_settings.target_gain_history,
                &mut self.target_gain_read_position,
            );
            if !self.paused {
                if self.trim_trace_enabled {
                    self.trim_plot.traces[0].ingest(&gain_samples, interval, now);
                    self.trim_plot.traces[1].ingest(&target_samples, interval, now);
                } else {
                    self.trim_plot.traces[0].points.clear();
                    self.trim_plot.traces[1].points.clear();
                }
            }
        }

        if !self.paused {
            self.signal_plot.trim(now);
            self.trim_plot.trim(now);
        }

        let link_group = egui::Id::new("audio_vis_linked");

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |cols| {
                // Left column: shared audio controls.
                let snapshot = self.snapshot();
                let mut commands = DirectAudioCommands {
                    settings: &self.processor_settings,
                    pending_device: None,
                };
                AudioPanel {
                    commands: &mut commands,
                    state: &mut self.audio_panel_state,
                    snapshot: &snapshot,
                }
                .ui(&mut cols[0]);

                self.pending_device = commands.pending_device.take();

                // Right column: display controls (visualization-specific).
                cols[1].strong("Display");
                if cols[1]
                    .button(if self.paused { "Resume" } else { "Pause" })
                    .clicked()
                {
                    self.paused = !self.paused;
                }
                egui::Grid::new("display_grid").show(&mut cols[1], |ui| {
                    ui.checkbox(&mut self.log_enabled, "Log transform");
                    if self.log_enabled {
                        ui.add(
                            egui::Slider::new(&mut self.log_range_db, 6.0..=60.0).suffix(" dB"),
                        );
                    }
                    ui.end_row();

                    ui.checkbox(&mut self.normalize_enabled, "Normalize traces");
                    ui.end_row();

                    ui.label("Window:");
                    let mut secs = self.signal_plot.window_seconds as f32;
                    if ui
                        .add(
                            egui::Slider::new(&mut secs, 0.5..=15.0)
                                .suffix(" s")
                                .logarithmic(true),
                        )
                        .changed()
                    {
                        self.signal_plot.window_seconds = secs as f64;
                        self.trim_plot.window_seconds = secs as f64;
                    }
                    ui.end_row();
                });
            });

            ui.separator();

            // Trace toggles.
            ui.horizontal_wrapped(|ui| {
                for (i, config) in SIGNAL_TRACE_CONFIGS.iter().enumerate() {
                    let mut enabled = self.signal_trace_enabled[i];
                    let label = egui::RichText::new(config.label).color(if enabled {
                        config.color
                    } else {
                        Color32::GRAY
                    });
                    if ui.checkbox(&mut enabled, label).changed() {
                        self.signal_trace_enabled[i] = enabled;
                    }
                }
                // Gain trace toggle.
                let label = egui::RichText::new("Gain").color(if self.trim_trace_enabled {
                    GAIN_TRACE_COLOR
                } else {
                    Color32::GRAY
                });
                ui.checkbox(&mut self.trim_trace_enabled, label);
                // Spectrum toggle + options.
                ui.checkbox(&mut self.spectrum_visible, "Spectrum");
                if self.spectrum_visible {
                    ui.checkbox(&mut self.spectrum_1f_weighted, "1/f");
                    ui.checkbox(&mut self.spectrum_log_freq, "Log Hz");
                    ui.add(
                        egui::Slider::new(&mut self.spectrum_y_max, 0.001..=1.0)
                            .text("Y")
                            .logarithmic(true),
                    );
                }
            });

            // Band steering parameter sliders.
            if self.spectrum_visible {
                ui.separator();
                ui.strong("Band Steering");
                egui::Grid::new("steering_grid").show(ui, |ui| {
                    let p = &self.steering_params;
                    let mut q = p.q.get();
                    if ui.add(egui::Slider::new(&mut q, 0.5..=20.0).text("Q").logarithmic(true)).changed() {
                        p.q.set(q);
                    }
                    ui.end_row();

                    // Expose as "speed" (1 - damping) on a log scale so the
                    // interesting range (0.001 to 0.1) gets proper resolution.
                    let speed = 1.0 - p.damping.get();
                    let mut speed_val = speed;
                    if ui.add(egui::Slider::new(&mut speed_val, 0.001..=1.0).text("Speed").logarithmic(true)).changed() {
                        p.damping.set(1.0 - speed_val);
                    }
                    ui.end_row();
                });
                if ui.button("Reset Filters").clicked() {
                    self.steering_params.request_reset();
                }
            }

            ui.separator();

            // Count visible plots.
            let any_signal = self.signal_trace_enabled.iter().any(|&e| e);
            let visible_plot_count = if any_signal { 1 } else { 0 }
                + if self.trim_trace_enabled { 1 } else { 0 }
                + if self.spectrum_visible { 1 } else { 0 };

            if visible_plot_count == 0 {
                ui.label("Enable a trace to see plots.");
            } else {
                let total_height = ui.available_height();
                // First plot gets half the space, rest split the remainder.
                let primary_height = if visible_plot_count == 1 {
                    total_height
                } else {
                    total_height * 0.5
                };
                let secondary_height = if visible_plot_count > 1 {
                    (total_height - primary_height) / (visible_plot_count - 1).max(1) as f32
                } else {
                    0.0
                };

                // Signal plot (main, 0-1 range).
                if any_signal {
                    let scales: Option<Vec<f32>> = if self.normalize_enabled {
                        Some(self.running_max.iter().map(|m| 1.0 / m).collect())
                    } else {
                        None
                    };

                    let height = if visible_plot_count == 1 || (!self.trim_trace_enabled && !self.spectrum_visible) {
                        total_height
                    } else {
                        primary_height
                    };

                    self.signal_plot.ui_with_options(
                        ui,
                        "audio_signals",
                        &self.signal_trace_enabled,
                        scales.as_deref(),
                        Some(link_group),
                        Some(height),
                        None,
                    );
                }

                // Gain plot (linked X, dB Y axis).
                if self.trim_trace_enabled {
                    use egui_plot::AxisHints;

                    let db_axis = AxisHints::new_y().formatter(|mark, _range| {
                        let db = 20.0 * (mark.value as f32).log10();
                        format!("{:+.0} dB", db)
                    });

                    self.trim_plot.ui_with_options(
                        ui,
                        "gain_plot",
                        &[true, true],
                        None,
                        Some(link_group),
                        Some(secondary_height),
                        Some(vec![db_axis]),
                    );
                }

                // Spectrum plot (frequency domain).
                if self.spectrum_visible {
                    let snap = self.spectral_snapshot.load();
                    let height = if !any_signal && !self.trim_trace_enabled {
                        total_height
                    } else {
                        secondary_height
                    };
                    Self::render_spectrum_plot(
                        ui,
                        &snap,
                        height,
                        self.spectrum_1f_weighted,
                        self.spectrum_log_freq,
                        self.spectrum_y_max,
                    );
                }
            }
        });

        // Peak info bar hidden — exploring mass-distribution approach.
        // self.render_spectrum_info(ctx);

        // Apply pending device change (from the GUI device selector).
        if let Some(device) = self.pending_device.take() {
            self._input = device.map(|name| {
                tunnels_audio::reconnect::ReconnectingInput::new(
                    name,
                    self.processor_settings.clone(),
                )
            });
        }

        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use tunnels_audio::processor::{Processor, ProcessorSettings};

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
        for config in &SIGNAL_TRACE_CONFIGS {
            plot.add_trace(config.label, config.color);
        }

        let update_interval = 1.0 / 1000.0;
        let enabled = [true; NUM_SIGNAL_TRACES];

        let proc_buffers: [&tunnels_audio::ring_buffer::SignalRingBuffer; NUM_SIGNAL_TRACES] = [
            &settings.input_peak_history,
            &settings.envelope_history,
            &settings.smoothed_history,
        ];

        for (i, buf) in proc_buffers.iter().enumerate() {
            let mut pos = 0_usize;
            let mut samples = Vec::new();
            buf.drain_into(&mut samples, &mut pos);
            let t = samples.len() as f64 * update_interval;
            plot.traces[i].ingest(&samples, update_interval, t);
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
                for (i, config) in SIGNAL_TRACE_CONFIGS.iter().enumerate() {
                    let label = egui::RichText::new(config.label).color(config.color);
                    ui.checkbox(&mut app.signal_trace_enabled[i], label);
                }
            });
            ui.separator();
            app.signal_plot
                .ui(ui, "audio_signals", &app.signal_trace_enabled, None);
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
