//! Spectral analysis using a pseudo-CQT (FFT + log-spaced filterbank).
//!
//! Computes a standard FFT then aggregates linear bins into logarithmically-
//! spaced CQT bins using triangular filterbank weights. This gives us
//! equal resolution per octave with a single cheap FFT operation.
//!
//! Runs on a dedicated thread, consuming raw audio samples from a channel.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;

use arc_swap::ArcSwap;
use rustfft::{FftPlanner, num_complex::Complex};

use crate::band_steering::{
    BandSteering, BandSteeringSnapshot, ScoringMode, SharedFilterFreqs, SharedSteeringParams,
};
use crate::processor::ProcessorSettings;

/// CQT configuration.
const MIN_FREQ: f32 = 160.0;
const MAX_FREQ: f32 = 20000.0;
pub const BINS_PER_OCTAVE: usize = 48;

/// FFT size.
const FFT_SIZE: usize = 16384;

/// Recompute every quarter-window (75% overlap).
const HOP_DIVISOR: usize = 4;

/// EMA coefficient for the magnitude average (~10s half-life at update rate).
const MAGNITUDE_EMA_COEFF: f32 = 0.997;

/// EMA coefficient for interest accumulation (~3s half-life).
const INTEREST_ACCUM_COEFF: f32 = 0.99;

/// Bounded channel capacity for audio buffers.
const CHANNEL_CAPACITY: usize = 64;

/// Return the compute hop size.
pub fn compute_hop() -> usize {
    FFT_SIZE / HOP_DIVISOR
}

/// A triangular filterbank mapping linear FFT bins to log-spaced CQT bins.
struct CqtFilterbank {
    /// Center frequency of each CQT bin.
    center_freqs: Vec<f32>,
    /// For each CQT bin: (start_fft_bin, weights).
    filters: Vec<(usize, Vec<f32>)>,
}

impl CqtFilterbank {
    fn new(sample_rate: f32, fft_size: usize) -> Self {
        let num_octaves = (MAX_FREQ / MIN_FREQ).log2();
        let num_bins = (num_octaves * BINS_PER_OCTAVE as f32).ceil() as usize;
        let bin_hz = sample_rate / fft_size as f32;
        let ratio = (2.0_f32).powf(1.0 / BINS_PER_OCTAVE as f32);

        let mut center_freqs = Vec::with_capacity(num_bins);
        let mut filters = Vec::with_capacity(num_bins);

        for i in 0..num_bins {
            let center = MIN_FREQ * ratio.powf(i as f32);
            if center > MAX_FREQ {
                break;
            }
            center_freqs.push(center);

            // Adjacent bin centers define the triangle edges.
            let lo_freq = MIN_FREQ * ratio.powf(i as f32 - 1.0);
            let hi_freq = MIN_FREQ * ratio.powf(i as f32 + 1.0);

            let lo_bin = (lo_freq / bin_hz).floor() as usize;
            let hi_bin = ((hi_freq / bin_hz).ceil() as usize).min(fft_size / 2);

            if lo_bin >= hi_bin {
                filters.push((0, vec![]));
                continue;
            }

            // Triangular weights: rise from lo_freq to center, fall from center to hi_freq.
            let mut weights = Vec::with_capacity(hi_bin - lo_bin);
            let mut weight_sum = 0.0_f32;
            for bin in lo_bin..hi_bin {
                let freq = bin as f32 * bin_hz;
                let w = if freq <= center {
                    if center <= lo_freq {
                        1.0
                    } else {
                        (freq - lo_freq) / (center - lo_freq)
                    }
                } else if hi_freq <= center {
                    1.0
                } else {
                    (hi_freq - freq) / (hi_freq - center)
                };
                let w = w.max(0.0);
                weights.push(w);
                weight_sum += w;
            }

            // Normalize weights to sum to 1 (output is weighted average).
            if weight_sum > 0.0 {
                for w in &mut weights {
                    *w /= weight_sum;
                }
            }

            filters.push((lo_bin, weights));
        }

        Self {
            center_freqs,
            filters,
        }
    }

    fn apply(&self, fft_magnitudes: &[f32]) -> Vec<f32> {
        self.filters
            .iter()
            .map(|(lo, weights)| {
                if weights.is_empty() {
                    return 0.0;
                }
                weights
                    .iter()
                    .enumerate()
                    .map(|(i, &w)| {
                        let bin = lo + i;
                        if bin < fft_magnitudes.len() {
                            w * fft_magnitudes[bin]
                        } else {
                            0.0
                        }
                    })
                    .sum()
            })
            .collect()
    }

    fn num_bins(&self) -> usize {
        self.center_freqs.len()
    }
}

/// Snapshot of spectral analysis results, shared with the GUI.
pub struct SpectralSnapshot {
    pub frequencies: Vec<f32>,
    pub magnitude: Vec<f32>,
    pub magnitude_avg: Vec<f32>,
    pub interest: Vec<f32>,
    pub interest_accum: Vec<f32>,
    pub interest_quality: Vec<f32>,
    pub spectral_contrast: Vec<f32>,
    /// The score surface actually fed to the steering (depends on scoring mode).
    pub steering_score: Vec<f32>,
    pub band_steering: BandSteeringSnapshot,
    pub sample_rate: f32,
}

impl Default for SpectralSnapshot {
    fn default() -> Self {
        Self {
            frequencies: Vec::new(),
            magnitude: Vec::new(),
            magnitude_avg: Vec::new(),
            interest: Vec::new(),
            interest_accum: Vec::new(),
            interest_quality: Vec::new(),
            spectral_contrast: Vec::new(),
            steering_score: Vec::new(),
            band_steering: BandSteeringSnapshot::default(),
            sample_rate: 48000.0,
        }
    }
}

pub type SharedSpectralSnapshot = Arc<ArcSwap<SpectralSnapshot>>;

pub fn new_shared_snapshot() -> SharedSpectralSnapshot {
    Arc::new(ArcSwap::from_pointee(SpectralSnapshot::default()))
}

pub fn start_spectral_thread(
    processor_settings: ProcessorSettings,
    sample_rate: f32,
    snapshot: SharedSpectralSnapshot,
    steering_params: Arc<SharedSteeringParams>,
    filter_freqs: Arc<SharedFilterFreqs>,
) -> Box<dyn FnOnce() + Send> {
    let (buf_tx, buf_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);

    {
        let mut guard = processor_settings.spectral_sender.lock().unwrap();
        *guard = Some(buf_tx);
    }

    let handle = thread::Builder::new()
        .name("spectral-analysis".into())
        .spawn(move || {
            run_spectral_loop(
                buf_rx,
                sample_rate,
                &snapshot,
                &steering_params,
                &filter_freqs,
            );
        })
        .expect("failed to spawn spectral analysis thread");

    Box::new(move || {
        handle.join().ok();
    })
}

fn run_spectral_loop(
    buf_rx: std::sync::mpsc::Receiver<Vec<f32>>,
    sample_rate: f32,
    snapshot_handle: &SharedSpectralSnapshot,
    steering_params: &Arc<SharedSteeringParams>,
    filter_freqs: &Arc<SharedFilterFreqs>,
) {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let filterbank = CqtFilterbank::new(sample_rate, FFT_SIZE);
    let num_bins = filterbank.num_bins();

    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32;
            0.5 * (1.0 - phase.cos())
        })
        .collect();

    let hop = compute_hop();
    let mut sample_buf: Vec<f32> = Vec::with_capacity(FFT_SIZE * 2);
    let mut samples_since_compute: usize = 0;
    let mut magnitude_avg: Vec<f32> = vec![0.0; num_bins];
    let mut interest_accum: Vec<f32> = vec![0.0; num_bins];
    let mut initialized = false;

    let mut band_steering = BandSteering::new();
    let dt = hop as f32 / sample_rate;

    while let Ok(buf) = buf_rx.recv() {
        sample_buf.extend_from_slice(&buf);
        samples_since_compute += buf.len();

        while sample_buf.len() > FFT_SIZE {
            sample_buf.drain(..sample_buf.len() - FFT_SIZE);
        }

        if samples_since_compute < hop || sample_buf.len() < FFT_SIZE {
            continue;
        }
        samples_since_compute = 0;

        // FFT.
        let mut fft_input: Vec<Complex<f32>> = sample_buf
            .iter()
            .zip(&window)
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();
        fft.process(&mut fft_input);

        let fft_mag: Vec<f32> = fft_input[..FFT_SIZE / 2 + 1]
            .iter()
            .map(|c| c.norm() / FFT_SIZE as f32)
            .collect();

        // Filterbank -> CQT magnitudes, then whiten (multiply by frequency).
        let magnitude: Vec<f32> = filterbank
            .apply(&fft_mag)
            .iter()
            .zip(&filterbank.center_freqs)
            .map(|(&mag, &freq)| mag * freq)
            .collect();

        // Magnitude EMA.
        if !initialized {
            magnitude_avg = magnitude.clone();
            initialized = true;
        } else {
            for (avg, &mag) in magnitude_avg.iter_mut().zip(&magnitude) {
                *avg = MAGNITUDE_EMA_COEFF * *avg + (1.0 - MAGNITUDE_EMA_COEFF) * mag;
            }
        }

        // Interest.
        let interest: Vec<f32> = magnitude
            .iter()
            .zip(&magnitude_avg)
            .map(|(&mag, &avg)| (mag - avg) * mag.sqrt())
            .collect();

        // Accumulate |interest|.
        for (acc, &v) in interest_accum.iter_mut().zip(&interest) {
            *acc = INTEREST_ACCUM_COEFF * *acc + (1.0 - INTEREST_ACCUM_COEFF) * v.abs();
        }

        // Interest*quality: rewards bins where energy is changing.
        let interest_quality: Vec<f32> = interest_accum
            .iter()
            .zip(&magnitude_avg)
            .map(
                |(&int, &mag)| {
                    if mag > 1e-10 { int * int / mag } else { 0.0 }
                },
            )
            .collect();

        // Spectral contrast: rewards bins that stand out above their
        // local neighborhood (±1 octave). Computed from the EMA-averaged
        // magnitude so it reflects sustained spectral peaks, not
        // frame-to-frame noise.
        let window = BINS_PER_OCTAVE;
        let spectral_contrast: Vec<f32> = (0..num_bins)
            .map(|i| {
                let lo = i.saturating_sub(window);
                let hi = (i + window + 1).min(num_bins);
                let local_mean = magnitude_avg[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
                if local_mean > 1e-10 {
                    magnitude_avg[i] / local_mean
                } else {
                    0.0
                }
            })
            .collect();

        // Select scoring metric based on GUI setting.
        let mode = ScoringMode::from_u32(steering_params.scoring_mode.load(Ordering::Relaxed));
        let steering_score: Vec<f32> = match mode {
            ScoringMode::InterestQuality => interest_quality.clone(),
            ScoringMode::SpectralContrast => spectral_contrast.clone(),
            ScoringMode::Blended => {
                let alpha = steering_params.blend_alpha.get();
                interest_quality
                    .iter()
                    .zip(&spectral_contrast)
                    .map(|(&iq, &sc)| alpha * iq + (1.0 - alpha) * sc)
                    .collect()
            }
        };

        // Band steering.
        band_steering.sync_params(steering_params);
        band_steering.update(&filterbank.center_freqs, &steering_score, sample_rate, dt);
        band_steering.write_shared_freqs(filter_freqs);

        snapshot_handle.store(Arc::new(SpectralSnapshot {
            frequencies: filterbank.center_freqs.clone(),
            magnitude,
            magnitude_avg: magnitude_avg.clone(),
            interest,
            interest_accum: interest_accum.clone(),
            interest_quality,
            spectral_contrast,
            steering_score,
            band_steering: band_steering.snapshot(),
            sample_rate,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn filterbank_bin_count() {
        let fb = CqtFilterbank::new(48000.0, FFT_SIZE);
        assert!(fb.num_bins() > 0);
        let expected = ((MAX_FREQ / MIN_FREQ).log2() * BINS_PER_OCTAVE as f32).ceil() as usize;
        assert!(fb.num_bins() <= expected);
    }

    #[test]
    fn filterbank_frequencies_are_log_spaced() {
        let fb = CqtFilterbank::new(48000.0, FFT_SIZE);
        assert!((fb.center_freqs[0] - MIN_FREQ).abs() < 0.01);

        let ratio = fb.center_freqs[1] / fb.center_freqs[0];
        for i in 1..fb.center_freqs.len() - 1 {
            let r = fb.center_freqs[i + 1] / fb.center_freqs[i];
            assert!((r - ratio).abs() < 0.001, "bin {}: {} vs {}", i, r, ratio);
        }
    }

    #[test]
    fn filterbank_weights_normalized() {
        let fb = CqtFilterbank::new(48000.0, FFT_SIZE);
        for (i, (_lo, weights)) in fb.filters.iter().enumerate() {
            if weights.is_empty() {
                continue;
            }
            let sum: f32 = weights.iter().sum();
            assert!((sum - 1.0).abs() < 0.01, "filter {} sums to {}", i, sum);
        }
    }

    #[test]
    fn pure_tone_peaks_at_correct_bin() {
        let sample_rate = 48000.0_f32;
        let fb = CqtFilterbank::new(sample_rate, FFT_SIZE);

        // Generate a 1000 Hz sine, compute FFT, apply filterbank.
        let freq = 1000.0_f32;
        let signal: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin())
            .collect();

        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let phase = 2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32;
                0.5 * (1.0 - phase.cos())
            })
            .collect();

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let mut fft_input: Vec<Complex<f32>> = signal
            .iter()
            .zip(&window)
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();
        fft.process(&mut fft_input);

        let fft_mag: Vec<f32> = fft_input[..FFT_SIZE / 2 + 1]
            .iter()
            .map(|c| c.norm() / FFT_SIZE as f32)
            .collect();

        let cqt_mag = fb.apply(&fft_mag);

        // Find the peak bin.
        let peak_bin = cqt_mag
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        let peak_freq = fb.center_freqs[peak_bin];

        assert!(
            (peak_freq - freq).abs() < 20.0,
            "Peak at {:.1} Hz, expected ~{:.1} Hz",
            peak_freq,
            freq,
        );
    }

    #[test]
    fn white_noise_is_approximately_flat() {
        // White noise should produce roughly equal energy per LINEAR Hz bin.
        // After the filterbank (which averages within each CQT bin), higher
        // bins average over more FFT bins but the weights are normalized,
        // so the output should be approximately flat.
        let fb = CqtFilterbank::new(48000.0, FFT_SIZE);

        // Simulate flat FFT magnitude (white noise).
        let flat_fft: Vec<f32> = vec![1.0; FFT_SIZE / 2 + 1];
        let cqt_mag = fb.apply(&flat_fft);

        // All bins should be approximately 1.0 since weights are normalized.
        for (i, &mag) in cqt_mag.iter().enumerate() {
            assert!(
                (mag - 1.0).abs() < 0.1,
                "bin {} ({}Hz): {:.3}, expected ~1.0",
                i,
                fb.center_freqs[i],
                mag,
            );
        }
    }

    #[test]
    fn benchmark_fft_plus_filterbank() {
        let sample_rate = 48000.0_f32;
        let fb = CqtFilterbank::new(sample_rate, FFT_SIZE);

        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let phase = 2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32;
                0.5 * (1.0 - phase.cos())
            })
            .collect();

        let signal: Vec<f32> = (0..FFT_SIZE).map(|i| (i as f32 * 0.1).sin()).collect();

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Warmup.
        for _ in 0..5 {
            let mut input: Vec<Complex<f32>> = signal
                .iter()
                .zip(&window)
                .map(|(&s, &w)| Complex::new(s * w, 0.0))
                .collect();
            fft.process(&mut input);
            let mag: Vec<f32> = input[..FFT_SIZE / 2 + 1].iter().map(|c| c.norm()).collect();
            fb.apply(&mag);
        }

        let iterations = 500;
        let start = Instant::now();
        for _ in 0..iterations {
            let mut input: Vec<Complex<f32>> = signal
                .iter()
                .zip(&window)
                .map(|(&s, &w)| Complex::new(s * w, 0.0))
                .collect();
            fft.process(&mut input);
            let mag: Vec<f32> = input[..FFT_SIZE / 2 + 1].iter().map(|c| c.norm()).collect();
            fb.apply(&mag);
        }
        let per_call = start.elapsed() / iterations;
        eprintln!(
            "FFT+filterbank: {:?} per call ({} CQT bins, {} FFT size)",
            per_call,
            fb.num_bins(),
            FFT_SIZE,
        );
    }
}
