pub mod scrolling_plot;

use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui::{self, Color32};
use egui_plot::{AxisHints, Line, Plot, PlotPoint, PlotPoints, VLine};

use gui_common::audio_panel::{AudioCommands, AudioPanel, AudioPanelState, AudioSnapshot};
use tunnels_audio::band_steering::{STEERED_FILTER_COUNT, SharedFilterFreqs};
use tunnels_audio::log_scale::DEFAULT_RANGE_DB;
use tunnels_audio::processor::ProcessorSettings;
use tunnels_audio::spectral::{self, SharedSpectralSnapshot};
use tunnels_audio::wavelet::{BAND_LABELS, NUM_BANDS};

use scrolling_plot::ScrollingPlot;

fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devs| devs.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// Create a cpal output stream that reads from the monitor ring buffer.
fn create_monitor_output(device_name: &str, settings: &ProcessorSettings) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .output_devices()
        .ok()?
        .find(|d| d.name().ok().as_deref() == Some(device_name))?;

    let config = device.default_output_config().ok()?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;

    let settings = Arc::clone(settings);
    // Start reading from a position slightly behind the current write
    // to pre-fill the jitter buffer (~50ms lead at 48kHz).
    let write_pos = settings.monitor_ring.write_pos();
    let mut read_pos = write_pos.wrapping_sub(2048.min(settings.monitor_ring.capacity() / 2));
    // Pre-allocate mono buffer for the largest expected callback size.
    let mut mono_buf = vec![0.0_f32; 4096];

    let stream = device
        .build_output_stream(
            &cpal::StreamConfig {
                channels: channels as u16,
                sample_rate: cpal::SampleRate(sample_rate),
                buffer_size: cpal::BufferSize::Default,
            },
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mono_frames = output.len() / channels;
                // Grow the pre-allocated buffer if needed (rare, only on first large callback).
                if mono_buf.len() < mono_frames {
                    mono_buf.resize(mono_frames, 0.0);
                }
                let mono = &mut mono_buf[..mono_frames];
                settings.monitor_ring.read_into_slice(mono, &mut read_pos);
                for (i, frame) in output.chunks_mut(channels).enumerate() {
                    let sample = mono[i];
                    for s in frame.iter_mut() {
                        *s = sample;
                    }
                }
            },
            |err| eprintln!("Monitor output error: {err}"),
            None,
        )
        .ok()?;

    stream.play().ok()?;
    Some(stream)
}

/// Approximate update rate of the envelope (once per audio buffer, ~1kHz).
const ENVELOPE_SAMPLE_RATE: f64 = 1000.0;

/// 3 base + 6 steered + 8 wavelet raw + 8 wavelet normalized.
const NUM_BASE_TRACES: usize = 3;
const NUM_STEERED_START: usize = NUM_BASE_TRACES;
const NUM_WAVELET_START: usize = NUM_BASE_TRACES + STEERED_FILTER_COUNT;
const NUM_WAVELET_NORM_START: usize = NUM_WAVELET_START + NUM_BANDS;
const NUM_SIGNAL_TRACES: usize = NUM_WAVELET_NORM_START + NUM_BANDS;

struct TraceConfig {
    label: &'static str,
    color: Color32,
    default_enabled: bool,
}

const SIGNAL_TRACE_CONFIGS: [TraceConfig; NUM_SIGNAL_TRACES] = [
    // Base traces.
    TraceConfig {
        label: "Input peak",
        color: Color32::from_rgb(80, 80, 100),
        default_enabled: false,
    },
    TraceConfig {
        label: "Lowpass",
        color: Color32::from_rgb(180, 180, 180),
        default_enabled: false,
    },
    TraceConfig {
        label: "N Lowpass",
        color: Color32::WHITE,
        default_enabled: true,
    },
    // Steered filter envelopes (off by default now that wavelet bands exist).
    TraceConfig {
        label: "LM-0",
        color: Color32::from_rgb(80, 160, 255),
        default_enabled: false,
    },
    TraceConfig {
        label: "LM-1",
        color: Color32::from_rgb(180, 100, 255),
        default_enabled: false,
    },
    TraceConfig {
        label: "Mid-0",
        color: Color32::from_rgb(255, 240, 60),
        default_enabled: false,
    },
    TraceConfig {
        label: "Mid-1",
        color: Color32::from_rgb(255, 160, 40),
        default_enabled: false,
    },
    TraceConfig {
        label: "Hi-0",
        color: Color32::from_rgb(60, 255, 180),
        default_enabled: false,
    },
    TraceConfig {
        label: "Hi-1",
        color: Color32::from_rgb(255, 80, 120),
        default_enabled: false,
    },
    // Wavelet band envelopes (raw) — rainbow, off by default.
    TraceConfig {
        label: "12-24k",
        color: Color32::from_rgb(255, 60, 60),
        default_enabled: false,
    },
    TraceConfig {
        label: "6-12k",
        color: Color32::from_rgb(255, 160, 40),
        default_enabled: false,
    },
    TraceConfig {
        label: "3-6k",
        color: Color32::from_rgb(255, 240, 60),
        default_enabled: false,
    },
    TraceConfig {
        label: "1.5-3k",
        color: Color32::from_rgb(60, 255, 120),
        default_enabled: false,
    },
    TraceConfig {
        label: "750-1.5k",
        color: Color32::from_rgb(60, 220, 255),
        default_enabled: false,
    },
    TraceConfig {
        label: "375-750",
        color: Color32::from_rgb(80, 120, 255),
        default_enabled: false,
    },
    TraceConfig {
        label: "187-375",
        color: Color32::from_rgb(180, 80, 255),
        default_enabled: false,
    },
    TraceConfig {
        label: "<187",
        color: Color32::from_rgb(160, 160, 160),
        default_enabled: false,
    },
    // Wavelet band envelopes (normalized) — same colors, on by default.
    TraceConfig {
        label: "N 12-24k",
        color: Color32::from_rgb(255, 60, 60),
        default_enabled: true,
    },
    TraceConfig {
        label: "N 6-12k",
        color: Color32::from_rgb(255, 160, 40),
        default_enabled: true,
    },
    TraceConfig {
        label: "N 3-6k",
        color: Color32::from_rgb(255, 240, 60),
        default_enabled: true,
    },
    TraceConfig {
        label: "N 1.5-3k",
        color: Color32::from_rgb(60, 255, 120),
        default_enabled: true,
    },
    TraceConfig {
        label: "N 750-1.5k",
        color: Color32::from_rgb(60, 220, 255),
        default_enabled: true,
    },
    TraceConfig {
        label: "N 375-750",
        color: Color32::from_rgb(80, 120, 255),
        default_enabled: true,
    },
    TraceConfig {
        label: "N 187-375",
        color: Color32::from_rgb(180, 80, 255),
        default_enabled: true,
    },
    TraceConfig {
        label: "N <187",
        color: Color32::from_rgb(160, 160, 160),
        default_enabled: true,
    },
];

const GAIN_TRACE_COLOR: Color32 = Color32::from_rgb(200, 160, 60);
const TARGET_GAIN_TRACE_COLOR: Color32 = Color32::from_rgb(120, 120, 180);

/// Adapter: AudioCommands by writing directly to ProcessorSettings atomics.
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
    signal_plot: ScrollingPlot,
    trim_plot: ScrollingPlot,
    signal_read_positions: [usize; NUM_SIGNAL_TRACES],
    gain_read_position: usize,
    target_gain_read_position: usize,
    signal_trace_enabled: [bool; NUM_SIGNAL_TRACES],
    trim_trace_enabled: bool,
    spectrum_visible: bool,
    spectrum_y_max: f32,
    start_time: Instant,
    log_range_db: f32,
    log_enabled: bool,
    normalize_enabled: bool,
    running_max: [f32; NUM_SIGNAL_TRACES],
    paused: bool,
    // === Monitor output ===
    output_devices: Vec<String>,
    selected_output: Option<String>,
    monitor_freq: f32,
    _monitor_stream: Option<cpal::Stream>,
}

impl AudioVisApp {
    pub fn new(processor_settings: ProcessorSettings) -> Self {
        let mut signal_plot = ScrollingPlot::new(3.0, 0.0, 1.1);
        let mut signal_trace_enabled = [false; NUM_SIGNAL_TRACES];

        for (i, config) in SIGNAL_TRACE_CONFIGS.iter().enumerate() {
            signal_plot.add_trace(config.label, config.color);
            signal_trace_enabled[i] = config.default_enabled;
        }

        let mut trim_plot = ScrollingPlot::new(3.0, 0.0, 3.5);
        trim_plot.add_trace("Effective gain", GAIN_TRACE_COLOR);
        trim_plot.add_trace("Target gain", TARGET_GAIN_TRACE_COLOR);

        // Read positions: base traces, steered envelopes, wavelet band envelopes.
        let mut signal_read_positions = [0_usize; NUM_SIGNAL_TRACES];
        signal_read_positions[0] = processor_settings.input_peak_history.write_pos();
        signal_read_positions[1] = processor_settings.smoothed_history.write_pos();
        signal_read_positions[2] = processor_settings.lowpass_norm_history.write_pos();
        for i in 0..STEERED_FILTER_COUNT {
            signal_read_positions[NUM_STEERED_START + i] =
                processor_settings.steered_envelope_histories[i].write_pos();
        }
        for i in 0..NUM_BANDS {
            signal_read_positions[NUM_WAVELET_START + i] =
                processor_settings.wavelet_histories[i].write_pos();
            signal_read_positions[NUM_WAVELET_NORM_START + i] =
                processor_settings.wavelet_norm_histories[i].write_pos();
        }
        let gain_read_position = processor_settings.effective_gain_history.write_pos();
        let target_gain_read_position = processor_settings.target_gain_history.write_pos();

        let devices = tunnels_audio::AudioInput::devices().unwrap_or_default();

        let spectral_snapshot = spectral::new_shared_snapshot();
        let steering_params = tunnels_audio::band_steering::SharedSteeringParams::new();
        let filter_freqs = SharedFilterFreqs::new();

        // Share filter freqs with the processor (audio callback will read them).
        {
            let mut guard = processor_settings.shared_filter_freqs.lock().unwrap();
            *guard = Some(filter_freqs.clone());
        }

        let spectral_stop = spectral::start_spectral_thread(
            processor_settings.clone(),
            48000.0,
            spectral_snapshot.clone(),
            steering_params.clone(),
            filter_freqs,
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
            spectrum_y_max: 0.001,
            start_time: Instant::now(),
            log_range_db: DEFAULT_RANGE_DB,
            log_enabled: false,
            normalize_enabled: false,
            running_max: [0.001; NUM_SIGNAL_TRACES],
            paused: false,
            output_devices: list_output_devices(),
            selected_output: None,
            monitor_freq: 1000.0,
            _monitor_stream: None,
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

    /// Render the CQT spectrum plot with interest*quality, score surface,
    /// filter positions, and kernel overlays.
    fn render_spectrum(
        ui: &mut egui::Ui,
        snap: &tunnels_audio::spectral::SpectralSnapshot,
        height: f32,
        y_max: f32,
        monitor_freq: f32,
    ) {
        if snap.frequencies.is_empty() {
            ui.label("Waiting for spectral data...");
            return;
        }

        // X axis: log frequency with Hz labels.
        let freq_to_x = |f: f32| -> f64 { (f.max(1.0) as f64).log10() };

        let hz_axis = AxisHints::new_x()
            .label("Frequency")
            .min_thickness(24.0)
            .formatter(|mark, _range| {
                let f = 10.0_f64.powf(mark.value);
                if f >= 1000.0 {
                    format!("{:.1}k", f / 1000.0)
                } else {
                    format!("{:.0}", f)
                }
            });

        Plot::new("spectrum_plot")
            .height(height)
            .include_y(0.0)
            .include_y(y_max as f64)
            .auto_bounds(egui::Vec2b::new(true, false))
            .reset()
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show_axes([true, true])
            .custom_x_axes(vec![hz_axis])
            .legend(egui_plot::Legend::default())
            .show(ui, |plot_ui| {
                // Helper to build a line from frequencies + values.
                let make_line = |data: &[f32], name: &str, color: Color32, width: f32| {
                    let points: Vec<PlotPoint> = snap
                        .frequencies
                        .iter()
                        .zip(data)
                        .map(|(&f, &v)| PlotPoint::new(freq_to_x(f), v as f64))
                        .collect();
                    Line::new(name.to_string(), PlotPoints::Owned(points))
                        .color(color)
                        .width(width)
                };

                // Raw CQT magnitude.
                plot_ui.line(make_line(
                    &snap.magnitude,
                    "Magnitude",
                    Color32::from_rgb(60, 60, 100),
                    1.0,
                ));

                // Magnitude average (EMA baseline).
                plot_ui.line(make_line(
                    &snap.magnitude_avg,
                    "Avg",
                    Color32::from_rgb(80, 80, 140),
                    1.0,
                ));

                // Instantaneous interest.
                let interest_abs: Vec<f32> = snap.interest.iter().map(|v| v.abs()).collect();
                plot_ui.line(make_line(
                    &interest_abs,
                    "Interest",
                    Color32::from_rgb(255, 120, 60),
                    1.0,
                ));

                // Accumulated interest.
                plot_ui.line(make_line(
                    &snap.interest_accum,
                    "Accum",
                    Color32::from_rgb(180, 255, 180),
                    1.0,
                ));

                // Interest*quality.
                plot_ui.line(make_line(
                    &snap.interest_quality,
                    "I*Q",
                    Color32::from_rgb(255, 100, 255),
                    1.0,
                ));

                // Spectral contrast.
                plot_ui.line(make_line(
                    &snap.spectral_contrast,
                    "Contrast",
                    Color32::from_rgb(255, 180, 60),
                    1.0,
                ));

                // Active steering score (what the filters actually follow).
                plot_ui.line(make_line(
                    &snap.steering_score,
                    "Score",
                    Color32::from_rgb(255, 255, 255),
                    1.5,
                ));

                // Score surface from band steering.
                let score_points: Vec<PlotPoint> = snap
                    .band_steering
                    .score_surface
                    .iter()
                    .map(|&(f, s)| PlotPoint::new(freq_to_x(f), s as f64))
                    .collect();
                if !score_points.is_empty() {
                    plot_ui.line(
                        Line::new("Score", PlotPoints::Owned(score_points))
                            .color(Color32::from_rgb(100, 255, 100))
                            .width(1.5),
                    );
                }

                // Lane boundaries.
                let lane_boundary_color = Color32::from_rgba_premultiplied(255, 255, 255, 40);
                let mut seen_boundaries = std::collections::HashSet::new();
                for filter in &snap.band_steering.filters {
                    for &boundary in &[filter.lane_min_hz, filter.lane_max_hz] {
                        let key = (boundary * 10.0) as u32;
                        if seen_boundaries.insert(key) {
                            plot_ui.vline(
                                VLine::new(format!("{:.0} Hz", boundary), freq_to_x(boundary))
                                    .color(lane_boundary_color)
                                    .width(1.0),
                            );
                        }
                    }
                }

                // Wavelet octave band boundaries.
                {
                    let wavelet_boundary_color =
                        Color32::from_rgba_premultiplied(100, 255, 100, 50);
                    let wavelet_band_colors: [Color32; NUM_BANDS] = [
                        Color32::from_rgb(255, 60, 60),
                        Color32::from_rgb(255, 160, 40),
                        Color32::from_rgb(255, 240, 60),
                        Color32::from_rgb(60, 255, 120),
                        Color32::from_rgb(60, 220, 255),
                        Color32::from_rgb(80, 120, 255),
                        Color32::from_rgb(180, 80, 255),
                        Color32::from_rgb(160, 160, 160),
                    ];
                    // Band edges: 24000, 12000, 6000, 3000, 1500, 750, 375, 187.5
                    let mut edge_hz = 24000.0_f32;
                    for band in 0..NUM_BANDS {
                        let lo_hz = edge_hz / 2.0;
                        // Draw a faint vertical line at each boundary.
                        if band < NUM_BANDS - 1 {
                            plot_ui.vline(
                                VLine::new(format!("{:.0} Hz", lo_hz), freq_to_x(lo_hz))
                                    .color(wavelet_boundary_color)
                                    .width(1.0),
                            );
                        }
                        // Shade the band region with its color at very low opacity.
                        let hi_x = freq_to_x(edge_hz);
                        let lo_x = freq_to_x(lo_hz);
                        let c = wavelet_band_colors[band];
                        let fill_color =
                            Color32::from_rgba_premultiplied(c.r() / 4, c.g() / 4, c.b() / 4, 20);
                        let rect_points = vec![
                            PlotPoint::new(lo_x, 0.0),
                            PlotPoint::new(lo_x, y_max as f64),
                            PlotPoint::new(hi_x, y_max as f64),
                            PlotPoint::new(hi_x, 0.0),
                        ];
                        plot_ui.line(
                            Line::new(
                                format!("wband {}", BAND_LABELS[band]),
                                PlotPoints::Owned(rect_points),
                            )
                            .color(fill_color)
                            .fill(0.0)
                            .width(0.0),
                        );
                        edge_hz = lo_hz;
                    }
                }

                // Filter response curves — colors match envelope traces.
                let filter_colors: [Color32; STEERED_FILTER_COUNT] = [
                    Color32::from_rgb(80, 160, 255),  // LM-0
                    Color32::from_rgb(180, 100, 255), // LM-1
                    Color32::from_rgb(255, 240, 60),  // Mid-0
                    Color32::from_rgb(255, 160, 40),  // Mid-1
                    Color32::from_rgb(60, 255, 180),  // Hi-0
                    Color32::from_rgb(255, 80, 120),  // Hi-1
                ];
                let q = snap.band_steering.q;
                let sample_rate = snap.sample_rate;

                let bins_per_octave = tunnels_audio::spectral::BINS_PER_OCTAVE as f32;
                let kernel_half = snap.band_steering.kernel_half_bins as f32;

                // Filter responses in dB, mapped to [0, y_max].
                // 0 dB → y_max, -60 dB → 0.
                const RESPONSE_FLOOR_DB: f32 = -40.0;
                let db_to_y = |linear: f32| -> f64 {
                    let db = if linear > 1e-10 {
                        20.0 * linear.log10()
                    } else {
                        RESPONSE_FLOOR_DB
                    };
                    let normalized = (db - RESPONSE_FLOOR_DB) / (0.0 - RESPONSE_FLOOR_DB);
                    (normalized.clamp(0.0, 1.0) * y_max) as f64
                };

                for (fi, filter) in snap.band_steering.filters.iter().enumerate() {
                    let color = filter_colors[fi % filter_colors.len()];

                    // RBJ bandpass response curve in dB (solid).
                    let response_points: Vec<PlotPoint> = snap
                        .frequencies
                        .iter()
                        .map(|&f| {
                            let r = filter.response_at(f, sample_rate, q);
                            PlotPoint::new(freq_to_x(f), db_to_y(r))
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

                    // Raised cosine convolution kernel in dB (dimmer).
                    if kernel_half > 0.0 {
                        let kernel_color = Color32::from_rgba_premultiplied(
                            color.r() / 2,
                            color.g() / 2,
                            color.b() / 2,
                            180,
                        );
                        let kernel_points: Vec<PlotPoint> = snap
                            .frequencies
                            .iter()
                            .map(|&f| {
                                let dist_bins =
                                    ((f / filter.center_hz).log2() * bins_per_octave).abs();
                                let weight = if dist_bins >= kernel_half {
                                    0.0
                                } else {
                                    0.5 * (1.0
                                        + (std::f32::consts::PI * dist_bins / kernel_half).cos())
                                };
                                PlotPoint::new(freq_to_x(f), db_to_y(weight))
                            })
                            .collect();
                        plot_ui.line(
                            Line::new(
                                format!("{:.0} Hz kernel", filter.center_hz),
                                PlotPoints::Owned(kernel_points),
                            )
                            .color(kernel_color)
                            .width(1.0),
                        );
                    }
                }

                // Monitor filter response (white, dashed-style via thinner line).
                if monitor_freq > 0.0 {
                    let monitor_snap = tunnels_audio::band_steering::FilterSnapshot {
                        center_hz: monitor_freq,
                        lane_min_hz: 0.0,
                        lane_max_hz: 0.0,
                        lane_index: 0,
                    };
                    let monitor_points: Vec<PlotPoint> = snap
                        .frequencies
                        .iter()
                        .map(|&f| {
                            let r = monitor_snap.response_at(f, sample_rate, q);
                            PlotPoint::new(freq_to_x(f), db_to_y(r))
                        })
                        .collect();
                    plot_ui.line(
                        Line::new(
                            format!("Monitor {:.0} Hz", monitor_freq),
                            PlotPoints::Owned(monitor_points),
                        )
                        .color(Color32::WHITE)
                        .width(2.0),
                    );
                }

                // Mask bounds around target peaks.
                let mask_half = snap.band_steering.mask_half_bins as f32;
                if mask_half > 0.0 {
                    let mask_color = Color32::from_rgba_premultiplied(255, 60, 60, 50);
                    for &(peak_hz, lane_idx) in &snap.band_steering.target_peaks {
                        let _ = lane_idx;
                        // Mask extends mask_half bins in each direction.
                        let lo_hz = peak_hz / 2.0_f32.powf(mask_half / bins_per_octave);
                        let hi_hz = peak_hz * 2.0_f32.powf(mask_half / bins_per_octave);
                        // Draw a filled rectangle from lo to hi at full Y.
                        let rect_points = vec![
                            PlotPoint::new(freq_to_x(lo_hz), 0.0),
                            PlotPoint::new(freq_to_x(lo_hz), y_max as f64),
                            PlotPoint::new(freq_to_x(hi_hz), y_max as f64),
                            PlotPoint::new(freq_to_x(hi_hz), 0.0),
                        ];
                        plot_ui.line(
                            Line::new(
                                format!("mask {:.0}", peak_hz),
                                PlotPoints::Owned(rect_points),
                            )
                            .color(mask_color)
                            .fill(0.0)
                            .width(0.5),
                        );
                    }
                }
            });
    }
}

impl eframe::App for AudioVisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = self.current_time();
        let interval = 1.0 / ENVELOPE_SAMPLE_RATE;

        let log_enabled = self.log_enabled;
        let log_range = self.log_range_db;

        // Drain signal traces (base + steered envelopes).
        let base_buffers: [&tunnels_audio::ring_buffer::SignalRingBuffer; NUM_BASE_TRACES] = [
            &self.processor_settings.input_peak_history,
            &self.processor_settings.smoothed_history,
            &self.processor_settings.lowpass_norm_history,
        ];
        let get_buf = |i: usize| -> &tunnels_audio::ring_buffer::SignalRingBuffer {
            if i < NUM_BASE_TRACES {
                base_buffers[i]
            } else if i < NUM_WAVELET_START {
                &self.processor_settings.steered_envelope_histories[i - NUM_STEERED_START]
            } else if i < NUM_WAVELET_NORM_START {
                &self.processor_settings.wavelet_histories[i - NUM_WAVELET_START]
            } else {
                &self.processor_settings.wavelet_norm_histories[i - NUM_WAVELET_NORM_START]
            }
        };

        for i in 0..NUM_SIGNAL_TRACES {
            let buf = get_buf(i);
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

        // Drain gain traces.
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
                // Left: shared audio controls.
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

                // Right: display controls.
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
                        ui.add(egui::Slider::new(&mut self.log_range_db, 6.0..=60.0).suffix(" dB"));
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
                    ui.label("AGC Floor:");
                    ui.horizontal(|ui| {
                        let mut floor_hl = self.processor_settings.norm_floor_halflife.get();
                        if ui
                            .add(
                                egui::Slider::new(&mut floor_hl, 0.5..=30.0)
                                    .logarithmic(true)
                                    .suffix("s"),
                            )
                            .changed()
                        {
                            self.processor_settings.norm_floor_halflife.set(floor_hl);
                        }
                        let cur = self
                            .processor_settings
                            .norm_floor_mode
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if ui.selectable_label(cur == 0, "Avg").clicked() {
                            self.processor_settings
                                .norm_floor_mode
                                .store(0, std::sync::atomic::Ordering::Relaxed);
                        }
                        if ui.selectable_label(cur == 1, "Min").clicked() {
                            self.processor_settings
                                .norm_floor_mode
                                .store(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    });
                    ui.end_row();
                    ui.label("AGC Ceil:");
                    ui.horizontal(|ui| {
                        let mut ceil_hl = self.processor_settings.norm_ceiling_halflife.get();
                        if ui
                            .add(
                                egui::Slider::new(&mut ceil_hl, 0.5..=15.0)
                                    .logarithmic(true)
                                    .suffix("s"),
                            )
                            .changed()
                        {
                            self.processor_settings.norm_ceiling_halflife.set(ceil_hl);
                        }
                        let cur = self
                            .processor_settings
                            .norm_ceiling_mode
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if ui.selectable_label(cur == 0, "Avg").clicked() {
                            self.processor_settings
                                .norm_ceiling_mode
                                .store(0, std::sync::atomic::Ordering::Relaxed);
                        }
                        if ui.selectable_label(cur == 1, "Max").clicked() {
                            self.processor_settings
                                .norm_ceiling_mode
                                .store(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    });
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
                {
                    let mut y = self.signal_plot.y_max as f32;
                    if ui
                        .add(
                            egui::Slider::new(&mut y, 0.01..=10.0)
                                .text("Y")
                                .logarithmic(true),
                        )
                        .changed()
                    {
                        self.signal_plot.y_max = y as f64;
                    }
                }
                let label = egui::RichText::new("Gain").color(if self.trim_trace_enabled {
                    GAIN_TRACE_COLOR
                } else {
                    Color32::GRAY
                });
                ui.checkbox(&mut self.trim_trace_enabled, label);
                ui.checkbox(&mut self.spectrum_visible, "Spectrum");
            });

            // Monitor output controls.
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Monitor");
                // Output device dropdown.
                let current_label = self.selected_output.as_deref().unwrap_or("Off");
                egui::ComboBox::from_id_salt("monitor_output")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.selected_output.is_none(), "Off")
                            .clicked()
                        {
                            self.selected_output = None;
                            self._monitor_stream = None;
                            self.processor_settings.monitor_freq.set(0.0);
                        }
                        for name in &self.output_devices {
                            let selected = self.selected_output.as_deref() == Some(name);
                            if ui.selectable_label(selected, name).clicked() && !selected {
                                if let Some(stream) =
                                    create_monitor_output(name, &self.processor_settings)
                                {
                                    self._monitor_stream = Some(stream);
                                    self.selected_output = Some(name.clone());
                                    self.processor_settings.monitor_freq.set(self.monitor_freq);
                                }
                            }
                        }
                    });
                if ui.button("↻").clicked() {
                    self.output_devices = list_output_devices();
                }
                // Frequency slider.
                if self.selected_output.is_some() {
                    if ui
                        .add(
                            egui::Slider::new(&mut self.monitor_freq, 20.0..=20000.0)
                                .text("Hz")
                                .logarithmic(true),
                        )
                        .changed()
                    {
                        self.processor_settings.monitor_freq.set(self.monitor_freq);
                    }
                }
            });

            ui.separator();

            // Plot layout.
            let any_signal = self.signal_trace_enabled.iter().any(|&e| e);
            let visible_count = if any_signal { 1 } else { 0 }
                + if self.trim_trace_enabled { 1 } else { 0 }
                + if self.spectrum_visible { 1 } else { 0 };

            if visible_count == 0 {
                ui.label("Enable a trace to see plots.");
            } else {
                let total_height = ui.available_height();
                let primary_height = if visible_count == 1 {
                    total_height
                } else {
                    total_height * 0.5
                };
                let secondary_height = if visible_count > 1 {
                    (total_height - primary_height) / (visible_count - 1).max(1) as f32
                } else {
                    0.0
                };

                if any_signal {
                    let scales: Option<Vec<f32>> = if self.normalize_enabled {
                        Some(self.running_max.iter().map(|m| 1.0 / m).collect())
                    } else {
                        None
                    };
                    let height = if visible_count == 1 {
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

                if self.trim_trace_enabled {
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

                if self.spectrum_visible {
                    // Spectrum controls — rendered inline above the plot.
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::Slider::new(&mut self.spectrum_y_max, 1e-12..=1000.0)
                                .text("Y")
                                .logarithmic(true),
                        );
                        let p = &self.steering_params;
                        let mut bw = 1.0 / p.q.get();
                        if ui
                            .add(
                                egui::Slider::new(&mut bw, 0.5..=2.0)
                                    .text("BW")
                                    .logarithmic(true),
                            )
                            .changed()
                        {
                            p.q.set(1.0 / bw);
                        }
                        let speed = 1.0 - p.damping.get();
                        let mut speed_val = speed;
                        if ui
                            .add(
                                egui::Slider::new(&mut speed_val, 0.001..=1.0)
                                    .text("Speed")
                                    .logarithmic(true),
                            )
                            .changed()
                        {
                            p.damping.set(1.0 - speed_val);
                        }
                        if ui.button("Reset").clicked() {
                            self.steering_params.request_reset();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        use tunnels_audio::band_steering::ScoringMode;
                        let p = &self.steering_params;
                        let current_mode = ScoringMode::from_u32(
                            p.scoring_mode.load(std::sync::atomic::Ordering::Relaxed),
                        );
                        ui.label("Score:");
                        for mode in ScoringMode::ALL {
                            if ui
                                .selectable_label(current_mode == mode, mode.label())
                                .clicked()
                            {
                                p.scoring_mode
                                    .store(mode as u32, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        if current_mode == ScoringMode::Blended {
                            let mut alpha = p.blend_alpha.get();
                            if ui
                                .add(egui::Slider::new(&mut alpha, 0.0..=1.0).text("α"))
                                .changed()
                            {
                                p.blend_alpha.set(alpha);
                            }
                        }
                        ui.separator();
                    });

                    let snap = self.spectral_snapshot.load();
                    let height = if !any_signal && !self.trim_trace_enabled {
                        total_height
                    } else {
                        secondary_height
                    };
                    Self::render_spectrum(
                        ui,
                        &snap,
                        height,
                        self.spectrum_y_max,
                        self.monitor_freq,
                    );
                }
            }
        });

        // Apply pending device change.
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
            if (t - 0.3).abs() < 1.0 / sr { 1.0 } else { 0.0 }
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
