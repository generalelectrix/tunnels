//! Slow-moving spectral analysis for identifying regions of dynamic interest.
//!
//! Runs on a dedicated thread, consuming raw audio samples from a ring buffer.
//! Produces:
//!   - Current FFT magnitude spectrum
//!   - "Spectral interest" per bin (where energy is *changing*, not just where it *is*)
//!   - Extracted centroid + bandwidth of the most interesting spectral region
//!
//! The spectral advisor updates at ~1-2 Hz with a 4096-point FFT (~85ms window
//! at 48kHz, ~11.7 Hz per bin).

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread;

use arc_swap::ArcSwap;
use rustfft::{Fft, FftPlanner, num_complex::Complex};

use crate::processor::ProcessorSettings;

/// FFT size. 16384 at 48kHz gives ~2.9 Hz per bin, ~341ms window.
/// Large enough for good bass resolution, cheap at our 2 Hz compute rate.
const FFT_SIZE: usize = 16384;

/// Minimum frequency for interest/quality computations.
/// Bins below this are zeroed to suppress subsonic artifacts.
const MIN_FREQ_HZ: f32 = 20.0;

/// Recompute the FFT every time this many new samples arrive.
/// One quarter of the FFT window = 75% overlap (standard for Hann windows).
/// At 48kHz this is ~85ms between FFTs, giving ~12 Hz update rate.
const COMPUTE_HOP: usize = FFT_SIZE / 4;

/// EMA coefficient for the spectral magnitude average.
/// Controls how quickly the "expected" spectrum adapts.
/// ~10s half-life at 20 Hz compute rate.
const MAGNITUDE_EMA_COEFF: f32 = 0.9965;

/// EMA coefficient for time-accumulating the interest curve.
/// ~3s half-life at ~12 Hz compute rate (COMPUTE_HOP-driven).
/// Captures "where has interest been hanging out recently."
const INTEREST_ACCUM_COEFF: f32 = 0.98;

/// EMA coefficient for smoothing the extracted centroid/bandwidth.
/// ~2s half-life at ~12 Hz — responsive but not jittery.
const CENTROID_SMOOTH_COEFF: f32 = 0.983;

/// A detected spectral peak with center frequency and bandwidth.
#[derive(Debug, Clone)]
pub struct SpectralPeak {
    /// Center frequency in Hz.
    pub center_hz: f32,
    /// Bandwidth in Hz (-3 dB width).
    pub bandwidth_hz: f32,
    /// Peak prominence (how much it stands out from surroundings).
    pub prominence: f32,
}

/// Snapshot of spectral analysis results, shared with the GUI.
pub struct SpectralSnapshot {
    /// Frequency in Hz for each bin (length = FFT_SIZE/2 + 1).
    pub frequencies: Vec<f32>,
    /// Current magnitude spectrum (linear scale).
    pub magnitude: Vec<f32>,
    /// Current magnitude with 1/f weighting (equal energy per octave).
    pub magnitude_weighted: Vec<f32>,
    /// Long-term average magnitude per bin.
    pub magnitude_avg: Vec<f32>,
    /// Long-term average with 1/f weighting.
    pub magnitude_avg_weighted: Vec<f32>,
    /// Spectral interest per bin (raw).
    pub interest: Vec<f32>,
    /// Interest with 1/f weighting.
    pub interest_weighted: Vec<f32>,
    /// Time-accumulated interest (EMA of |interest| over ~3s).
    /// This is what peak finding operates on — shows where dynamics
    /// have been concentrated recently, not just right now.
    pub interest_accum: Vec<f32>,
    /// Time-accumulated 1/f-weighted interest.
    pub interest_accum_weighted: Vec<f32>,
    /// Detected peaks from accumulated interest, sorted by prominence.
    pub peaks: Vec<SpectralPeak>,
    /// Detected peaks from accumulated 1/f-weighted interest.
    pub peaks_weighted: Vec<SpectralPeak>,
    /// Sample rate used for frequency calculations.
    pub sample_rate: f32,
}

impl Default for SpectralSnapshot {
    fn default() -> Self {
        Self {
            frequencies: Vec::new(),
            magnitude: Vec::new(),
            magnitude_weighted: Vec::new(),
            magnitude_avg: Vec::new(),
            magnitude_avg_weighted: Vec::new(),
            interest: Vec::new(),
            interest_weighted: Vec::new(),
            interest_accum: Vec::new(),
            interest_accum_weighted: Vec::new(),
            peaks: Vec::new(),
            peaks_weighted: Vec::new(),
            sample_rate: 48000.0,
        }
    }
}

pub type SharedSpectralSnapshot = Arc<ArcSwap<SpectralSnapshot>>;

pub fn new_shared_snapshot() -> SharedSpectralSnapshot {
    Arc::new(ArcSwap::from_pointee(SpectralSnapshot::default()))
}

/// Bounded channel capacity for audio buffers. Enough to absorb jitter
/// without unbounded growth. If full, the audio thread drops the buffer.
const CHANNEL_CAPACITY: usize = 64;

/// Start the spectral analysis thread.
///
/// Installs a channel sender into `processor_settings` that the audio
/// callback will use to send mono buffers. Returns a stop function.
pub fn start_spectral_thread(
    processor_settings: ProcessorSettings,
    sample_rate: f32,
    snapshot: SharedSpectralSnapshot,
) -> Box<dyn FnOnce() + Send> {
    let (buf_tx, buf_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);

    // Install the sender into ProcessorSettings so the audio callback can use it.
    {
        let mut guard = processor_settings.spectral_sender.lock().unwrap();
        *guard = Some(buf_tx);
    }

    let handle = thread::Builder::new()
        .name("spectral-analysis".into())
        .spawn(move || {
            run_spectral_loop(buf_rx, sample_rate, &snapshot);
        })
        .expect("failed to spawn spectral analysis thread");

    Box::new(move || {
        // Dropping the sender side will cause the recv to return Disconnected.
        // We already moved buf_tx into settings, so dropping settings will close it.
        // For explicit shutdown, we just join.
        handle.join().ok();
    })
}

fn run_spectral_loop(
    buf_rx: std::sync::mpsc::Receiver<Vec<f32>>,
    sample_rate: f32,
    snapshot_handle: &SharedSpectralSnapshot,
) {
    let mut analyzer = SpectralAnalyzer::new(sample_rate);

    // Event-driven: block on receiving each buffer from the audio thread.
    while let Ok(buf) = buf_rx.recv() {
        let should_compute = analyzer.push_samples(&buf);
        if should_compute {
            if let Some(result) = analyzer.compute() {
                snapshot_handle.store(Arc::new(result));
            }
        }
    }
}

/// Maximum number of peaks to extract.
const MAX_PEAKS: usize = 7;

/// Minimum bin separation between peaks (~30 Hz at our resolution).
const MIN_PEAK_DISTANCE: usize = 10;

/// Interest smoothing kernel half-width in bins. The kernel width scales
/// with frequency to approximate log-frequency smoothing: narrow in the
/// bass (preserve detail), wider in the treble (suppress noise).
const SMOOTH_BASE_HALFWIDTH: usize = 2;
const SMOOTH_MAX_HALFWIDTH: usize = 40;

struct SpectralAnalyzer {
    sample_rate: f32,
    fft: Arc<dyn Fft<f32>>,
    /// Accumulator for incoming samples (sliding window).
    sample_buf: VecDeque<f32>,
    /// Hann window coefficients.
    window: Vec<f32>,
    /// Per-bin frequency values.
    frequencies: Vec<f32>,
    /// Per-bin 1/f weight (clamped, for frequency normalization).
    freq_weights: Vec<f32>,
    /// Long-term EMA of magnitude per bin.
    magnitude_avg: Vec<f32>,
    /// Time-accumulated interest (EMA of absolute interest per bin).
    interest_accum: Vec<f32>,
    /// Time-accumulated 1/f-weighted interest.
    interest_accum_weighted: Vec<f32>,
    /// Smoothed peaks (EMA on center/bandwidth for temporal stability).
    smoothed_peaks: Vec<SpectralPeak>,
    smoothed_peaks_weighted: Vec<SpectralPeak>,
    /// Samples pushed since last compute.
    samples_since_compute: usize,
    /// Whether we've seen enough data to produce meaningful averages.
    initialized: bool,
}

impl SpectralAnalyzer {
    fn new(sample_rate: f32) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let phase = 2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32;
                0.5 * (1.0 - phase.cos()) // Hann window
            })
            .collect();

        let num_bins = FFT_SIZE / 2 + 1;
        let bin_width = sample_rate / FFT_SIZE as f32;
        let frequencies: Vec<f32> = (0..num_bins).map(|i| i as f32 * bin_width).collect();
        let freq_weights: Vec<f32> = frequencies
            .iter()
            .map(|&f| if f > 1.0 { f } else { 1.0 })
            .collect();

        Self {
            sample_rate,
            fft,
            sample_buf: VecDeque::with_capacity(FFT_SIZE * 2),
            window,
            frequencies,
            freq_weights,
            samples_since_compute: 0,
            magnitude_avg: vec![0.0; num_bins],
            interest_accum: vec![0.0; num_bins],
            interest_accum_weighted: vec![0.0; num_bins],
            smoothed_peaks: Vec::new(),
            smoothed_peaks_weighted: Vec::new(),
            initialized: false,
        }
    }

    /// Push new samples into the sliding window.
    /// Returns true if enough new samples have arrived to warrant a new FFT.
    fn push_samples(&mut self, samples: &[f32]) -> bool {
        self.sample_buf.extend(samples);
        self.samples_since_compute += samples.len();
        // Keep only the most recent FFT_SIZE samples.
        while self.sample_buf.len() > FFT_SIZE {
            self.sample_buf.pop_front();
        }
        self.samples_since_compute >= COMPUTE_HOP
    }

    /// Compute the FFT on the current window and return a snapshot.
    fn compute(&mut self) -> Option<SpectralSnapshot> {
        self.samples_since_compute = 0;
        if self.sample_buf.len() < FFT_SIZE {
            return None;
        }

        let magnitude = self.compute_magnitude();
        let interest = self.compute_interest(&magnitude);

        // 1/f weighted variants.
        let magnitude_weighted: Vec<f32> = magnitude
            .iter()
            .zip(&self.freq_weights)
            .map(|(&m, &w)| m * w)
            .collect();
        let magnitude_avg_weighted: Vec<f32> = self
            .magnitude_avg
            .iter()
            .zip(&self.freq_weights)
            .map(|(&m, &w)| m * w)
            .collect();
        let interest_weighted: Vec<f32> = interest
            .iter()
            .zip(&self.freq_weights)
            .map(|(&v, &w)| v * w)
            .collect();

        // Time-accumulate the absolute interest (EMA).
        for (acc, &v) in self.interest_accum.iter_mut().zip(&interest) {
            *acc = INTEREST_ACCUM_COEFF * *acc + (1.0 - INTEREST_ACCUM_COEFF) * v.abs();
        }
        for (acc, &v) in self.interest_accum_weighted.iter_mut().zip(&interest_weighted) {
            *acc = INTEREST_ACCUM_COEFF * *acc + (1.0 - INTEREST_ACCUM_COEFF) * v.abs();
        }

        // Smooth the accumulated interest spatially, then peak-find.
        let accum_smoothed = self.smooth_interest(&self.interest_accum.clone());
        let accum_weighted_smoothed = self.smooth_interest(&self.interest_accum_weighted.clone());

        let peaks = self.find_peaks(&accum_smoothed, false);
        let peaks_weighted = self.find_peaks(&accum_weighted_smoothed, true);

        Some(SpectralSnapshot {
            frequencies: self.frequencies.clone(),
            magnitude,
            magnitude_weighted,
            magnitude_avg: self.magnitude_avg.clone(),
            magnitude_avg_weighted,
            interest,
            interest_weighted,
            interest_accum: accum_smoothed,
            interest_accum_weighted: accum_weighted_smoothed,
            peaks,
            peaks_weighted,
            sample_rate: self.sample_rate,
        })
    }

    fn compute_magnitude(&mut self) -> Vec<f32> {
        let num_bins = FFT_SIZE / 2 + 1;

        // Apply window and convert to complex.
        let mut fft_input: Vec<Complex<f32>> = self
            .sample_buf
            .iter()
            .zip(&self.window)
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        self.fft.process(&mut fft_input);

        // Compute magnitude for positive frequencies.
        let scale = 1.0 / FFT_SIZE as f32;
        let magnitude: Vec<f32> = fft_input[..num_bins]
            .iter()
            .map(|c| c.norm() * scale)
            .collect();

        // Update EMA average.
        if !self.initialized {
            self.magnitude_avg = magnitude.clone();
            self.initialized = true;
        } else {
            for (avg, &mag) in self.magnitude_avg.iter_mut().zip(&magnitude) {
                *avg = MAGNITUDE_EMA_COEFF * *avg + (1.0 - MAGNITUDE_EMA_COEFF) * mag;
            }
        }

        magnitude
    }

    fn compute_interest(&self, magnitude: &[f32]) -> Vec<f32> {
        // Interest = magnitude of change, weighted by signal level.
        // We want bins that are both *changing* and have *meaningful energy*.
        // Raw deviation (mag - avg) captures the change in absolute terms.
        // Multiplying by sqrt(mag) biases toward bins with actual energy,
        // suppressing noise in quiet bins where tiny fluctuations produce
        // large ratios.
        // Bins below MIN_FREQ_HZ are zeroed to suppress subsonic artifacts.
        magnitude
            .iter()
            .zip(&self.magnitude_avg)
            .zip(&self.frequencies)
            .map(|((&mag, &avg), &freq)| {
                if freq < MIN_FREQ_HZ {
                    return 0.0;
                }
                let deviation = mag - avg;
                deviation * mag.sqrt()
            })
            .collect()
    }

    /// Smooth the interest curve with a variable-width moving average.
    /// Narrow in the bass (preserve detail where bins are already coarse
    /// relative to musical intervals), wider in the treble (suppress noise
    /// where we have far more bins than we need).
    fn smooth_interest(&self, interest: &[f32]) -> Vec<f32> {
        let n = interest.len();
        let mut smoothed = vec![0.0_f32; n];
        let bin_width = self.sample_rate / FFT_SIZE as f32;

        for i in 0..n {
            let freq = self.frequencies[i];
            // Scale kernel half-width logarithmically with frequency.
            // At 50 Hz: ~SMOOTH_BASE_HALFWIDTH bins.
            // At 20 kHz: ~SMOOTH_MAX_HALFWIDTH bins.
            let log_scale = if freq > 20.0 {
                ((freq / 50.0).log2().max(0.0) / (20000.0_f32 / 50.0).log2()) as usize
            } else {
                0
            };
            let half_w = SMOOTH_BASE_HALFWIDTH
                + log_scale * (SMOOTH_MAX_HALFWIDTH - SMOOTH_BASE_HALFWIDTH)
                    / ((20000.0 / bin_width) as usize).max(1);
            let half_w = half_w.min(SMOOTH_MAX_HALFWIDTH);

            let lo = i.saturating_sub(half_w);
            let hi = (i + half_w + 1).min(n);
            let count = (hi - lo) as f32;
            let sum: f32 = interest[lo..hi].iter().map(|v| v.abs()).sum();
            smoothed[i] = sum / count;
        }
        smoothed
    }

    /// Find peaks in the smoothed interest curve using prominence-based detection.
    fn find_peaks(&mut self, interest_smoothed: &[f32], weighted: bool) -> Vec<SpectralPeak> {
        use find_peaks::PeakFinder;

        if interest_smoothed.is_empty() {
            return Vec::new();
        }

        // Find the rolling max for adaptive thresholding.
        let max_interest = interest_smoothed
            .iter()
            .fold(0.0_f32, |a, &b| a.max(b));

        if max_interest < 1e-10 {
            return Vec::new();
        }

        // Prominence threshold: 10% of the max interest.
        let min_prominence = max_interest * 0.10;

        let mut finder = PeakFinder::new(interest_smoothed);
        finder
            .with_min_prominence(min_prominence)
            .with_min_distance(MIN_PEAK_DISTANCE);
        let raw_peaks = finder.find_peaks();

        // Convert to SpectralPeaks with bandwidth estimation, sort by prominence.
        let bin_width = self.sample_rate / FFT_SIZE as f32;
        let mut peaks: Vec<SpectralPeak> = raw_peaks
            .iter()
            .map(|p| {
                let bin = p.middle_position();
                let center_hz = self.frequencies[bin.min(self.frequencies.len() - 1)];
                let prominence = p.prominence.unwrap_or(0.0);

                // Estimate bandwidth via -3dB (half-height) width on the smoothed curve.
                let peak_val = interest_smoothed[bin];
                let half_height = peak_val * 0.5;

                // Search left for half-height crossing.
                let mut left_bin = bin;
                while left_bin > 0 && interest_smoothed[left_bin] > half_height {
                    left_bin -= 1;
                }
                // Search right for half-height crossing.
                let mut right_bin = bin;
                while right_bin < interest_smoothed.len() - 1
                    && interest_smoothed[right_bin] > half_height
                {
                    right_bin += 1;
                }

                let bandwidth_hz = ((right_bin - left_bin) as f32 * bin_width).max(bin_width * 2.0);

                SpectralPeak {
                    center_hz,
                    bandwidth_hz,
                    prominence,
                }
            })
            .collect();

        // Sort by prominence descending, keep top N.
        peaks.sort_by(|a, b| b.prominence.partial_cmp(&a.prominence).unwrap());
        peaks.truncate(MAX_PEAKS);

        // Temporal smoothing: match new peaks to previous peaks by proximity
        // and smooth their center/bandwidth.
        let prev = if weighted {
            &self.smoothed_peaks_weighted
        } else {
            &self.smoothed_peaks
        };
        let smoothed = Self::smooth_peaks(prev, &peaks);
        if weighted {
            self.smoothed_peaks_weighted = smoothed.clone();
        } else {
            self.smoothed_peaks = smoothed.clone();
        }
        smoothed
    }

    /// Match new peaks to previous peaks by frequency proximity and apply EMA smoothing.
    fn smooth_peaks(prev_peaks: &[SpectralPeak], new_peaks: &[SpectralPeak]) -> Vec<SpectralPeak> {
        let mut result = Vec::with_capacity(new_peaks.len());

        for new in new_peaks {
            let match_threshold = new.bandwidth_hz.max(50.0);
            let matched = prev_peaks.iter().find(|old| {
                (old.center_hz - new.center_hz).abs() < match_threshold
            });

            let smoothed = if let Some(old) = matched {
                SpectralPeak {
                    center_hz: CENTROID_SMOOTH_COEFF * old.center_hz
                        + (1.0 - CENTROID_SMOOTH_COEFF) * new.center_hz,
                    bandwidth_hz: CENTROID_SMOOTH_COEFF * old.bandwidth_hz
                        + (1.0 - CENTROID_SMOOTH_COEFF) * new.bandwidth_hz,
                    prominence: new.prominence,
                }
            } else {
                new.clone()
            };
            result.push(smoothed);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sine_samples(freq_hz: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate * duration_secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq_hz * t).sin()
            })
            .collect()
    }

    #[test]
    fn pure_tone_centroid_near_frequency() {
        let sample_rate = 48000.0;
        let freq = 440.0;
        let mut analyzer = SpectralAnalyzer::new(sample_rate);

        // Feed enough for several FFT windows so the EMA settles.
        let samples = make_sine_samples(freq, sample_rate, 2.0);
        let chunk_size = 4800; // 100ms chunks
        let mut last_snapshot = None;
        for chunk in samples.chunks(chunk_size) {
            analyzer.push_samples(chunk);
            if let Some(s) = analyzer.compute() {
                last_snapshot = Some(s);
            }
        }

        let snap = last_snapshot.expect("should have produced a snapshot");

        // The magnitude peak should be near the 440 Hz bin.
        let peak_bin = snap
            .magnitude
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        let peak_freq = snap.frequencies[peak_bin];
        assert!(
            (peak_freq - freq).abs() < 20.0,
            "Peak frequency {:.1} should be near {:.1}",
            peak_freq,
            freq
        );
    }

    #[test]
    fn changing_tone_produces_interest() {
        let sample_rate = 48000.0;
        let mut analyzer = SpectralAnalyzer::new(sample_rate);

        // Feed 200 Hz for 1s to establish baseline.
        let baseline = make_sine_samples(200.0, sample_rate, 1.0);
        for chunk in baseline.chunks(4800) {
            analyzer.push_samples(chunk);
            analyzer.compute();
        }

        // Now switch to 1000 Hz — should produce interest around 1000 Hz.
        let changed = make_sine_samples(1000.0, sample_rate, 0.5);
        let mut last_snapshot = None;
        for chunk in changed.chunks(4800) {
            analyzer.push_samples(chunk);
            if let Some(s) = analyzer.compute() {
                last_snapshot = Some(s);
            }
        }

        let snap = last_snapshot.expect("should have produced a snapshot");

        // Find the bin with highest positive interest.
        let most_interesting_bin = snap
            .interest
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        let interesting_freq = snap.frequencies[most_interesting_bin];

        assert!(
            (interesting_freq - 1000.0).abs() < 50.0,
            "Most interesting frequency {:.1} should be near 1000 Hz after tone change",
            interesting_freq,
        );
    }

    #[test]
    fn silence_produces_no_interest() {
        let sample_rate = 48000.0;
        let mut analyzer = SpectralAnalyzer::new(sample_rate);

        let silence = vec![0.0_f32; 48000];
        let mut last_snapshot = None;
        for chunk in silence.chunks(4800) {
            analyzer.push_samples(chunk);
            if let Some(s) = analyzer.compute() {
                last_snapshot = Some(s);
            }
        }

        let snap = last_snapshot.expect("should have produced a snapshot");
        let max_interest = snap
            .interest
            .iter()
            .map(|v| v.abs())
            .fold(0.0_f32, f32::max);

        assert!(
            max_interest < 0.1,
            "Silence should produce near-zero interest, got {:.3}",
            max_interest
        );
    }

    #[test]
    fn steady_tone_produces_low_interest() {
        let sample_rate = 48000.0;
        let mut analyzer = SpectralAnalyzer::new(sample_rate);

        // Feed the same 300 Hz tone for 10 seconds — after the EMA settles,
        // interest should be low because nothing is changing.
        let samples = make_sine_samples(300.0, sample_rate, 10.0);
        let mut last_snapshot = None;
        for chunk in samples.chunks(4800) {
            analyzer.push_samples(chunk);
            if let Some(s) = analyzer.compute() {
                last_snapshot = Some(s);
            }
        }

        let snap = last_snapshot.expect("should have produced a snapshot");
        let max_interest = snap
            .interest
            .iter()
            .map(|v| v.abs())
            .fold(0.0_f32, f32::max);

        assert!(
            max_interest < 0.01,
            "Steady tone should produce near-zero interest after settling, got {:.6}",
            max_interest
        );
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    #[test]
    fn fft_timing() {
        for &size in &[4096_usize, 8192, 16384, 32768] {
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(size);
            let template: Vec<Complex<f32>> = (0..size)
                .map(|i| Complex::new((i as f32 * 0.1).sin(), 0.0))
                .collect();

            // Warmup
            for _ in 0..10 {
                let mut b = template.clone();
                fft.process(&mut b);
            }

            let iters = 500;
            let start = Instant::now();
            for _ in 0..iters {
                let mut b = template.clone();
                fft.process(&mut b);
            }
            let per_call = start.elapsed() / iters;
            eprintln!("{size:6} points: {per_call:?}");
        }
    }
}
