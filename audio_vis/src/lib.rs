pub mod scrolling_plot;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};

use gui_common::audio_panel::{AudioCommands, AudioPanel, AudioPanelState, AudioSnapshot};
use tunnels_audio::log_scale::DEFAULT_RANGE_DB;
use tunnels_audio::processor::ProcessorSettings;

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
        default_enabled: true,
    },
    TraceConfig {
        label: "Envelope",
        color: Color32::from_rgb(70, 180, 130),
        default_enabled: true,
    },
    TraceConfig {
        label: "Smoothed",
        color: Color32::WHITE,
        default_enabled: true,
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

        Self {
            processor_settings,
            _input: None,
            pending_device: None,
            audio_panel_state: AudioPanelState::new(devices),
            signal_plot,
            trim_plot,
            signal_read_positions,
            gain_read_position,
            target_gain_read_position,
            signal_trace_enabled,
            trim_trace_enabled: false,
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
            });

            ui.separator();

            // Signal plot (main, 0-1 range).
            let scales: Option<Vec<f32>> = if self.normalize_enabled {
                Some(self.running_max.iter().map(|m| 1.0 / m).collect())
            } else {
                None
            };

            let signal_height = if self.trim_trace_enabled {
                ui.available_height() * 0.75
            } else {
                ui.available_height()
            };

            self.signal_plot.ui_with_options(
                ui,
                "audio_signals",
                &self.signal_trace_enabled,
                scales.as_deref(),
                Some(link_group),
                Some(signal_height),
                None,
            );

            // Gain plot (linked X, dB Y axis) — only if enabled.
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
                    None, // takes remaining height
                    Some(vec![db_axis]),
                );
            }
        });

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
