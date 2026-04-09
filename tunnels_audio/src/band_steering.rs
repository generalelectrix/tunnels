//! Adaptive band steering via convolution + greedy assignment.
//!
//! Computes a "score surface" by convolving a triangle kernel with the
//! interest*quality distribution. The peaks of the score surface are the
//! optimal filter placements. Filters are assigned to peaks greedily,
//! with masking to prevent pile-up, and smoothed via EMA in log-frequency
//! (bin-index) space.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use audio_processor_traits::AtomicF32;

/// Total number of steered filters across all lanes (2 + 2 + 2).
pub const STEERED_FILTER_COUNT: usize = 6;

/// Per-filter info: (lane_index, filter_index_within_lane).
/// Matches the iteration order in BandSteering.
pub const STEERED_FILTER_LAYOUT: [(usize, usize); STEERED_FILTER_COUNT] = [
    (0, 0),
    (0, 1), // low-mid lane, 2 filters
    (1, 0),
    (1, 1), // mid lane, 2 filters
    (2, 0),
    (2, 1), // high lane, 2 filters
];

/// Shared atomic state for steered filter frequencies — written by the
/// spectral thread, read by the audio callback to configure bandpass filters.
pub struct SharedFilterFreqs {
    pub center_hz: [AtomicF32; STEERED_FILTER_COUNT],
    pub q: AtomicF32,
}

impl SharedFilterFreqs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            center_hz: std::array::from_fn(|_| AtomicF32::new(0.0)),
            q: AtomicF32::new(DEFAULT_Q),
        })
    }
}

/// Configuration for a lane — a frequency region containing steerable filters.
#[derive(Debug, Clone)]
pub struct LaneConfig {
    pub min_hz: f32,
    pub max_hz: f32,
    pub filter_count: usize,
}

/// A single steerable filter within a lane.
#[derive(Debug, Clone)]
pub struct SteerableFilter {
    pub center_hz: f32,
    /// Last assigned target bin index (within the lane's bin range). Used for
    /// sticky assignment — the filter keeps its target unless a competing
    /// target's score is significantly better.
    last_target_bin: Option<usize>,
}

/// A lane containing one or more steerable filters.
#[derive(Debug, Clone)]
pub struct Lane {
    pub config: LaneConfig,
    pub filters: Vec<SteerableFilter>,
    /// Target bins from the previous frame (lane-local indices).
    prev_targets: Vec<usize>,
}

/// The complete band steering state.
pub struct BandSteering {
    pub lanes: Vec<Lane>,
    /// Q factor for the bandpass filters (controls triangle kernel width).
    pub q: f32,
    /// Smoothing factor for filter movement (0 = instant, 1 = frozen).
    pub damping: f32,
    /// Score surface from the most recent update.
    score_surface: Vec<(f32, f32)>,
    /// Target peaks from the most recent greedy assignment (before smoothing).
    target_peaks: Vec<(f32, usize)>,
    /// Convolution kernel half-width in bins from the most recent update.
    kernel_half_bins: usize,
    /// Exclusion mask half-width in bins.
    mask_half_bins: usize,
}

/// Snapshot of filter positions for the GUI.
#[derive(Debug, Clone)]
pub struct BandSteeringSnapshot {
    pub filters: Vec<FilterSnapshot>,
    /// Score surface: (frequency, score) pairs for visualization.
    pub score_surface: Vec<(f32, f32)>,
    /// Target peaks found by greedy masking (before smoothing).
    /// Each entry: (center_hz, lane_index).
    pub target_peaks: Vec<(f32, usize)>,
    /// Convolution kernel half-width in CQT bins.
    pub kernel_half_bins: usize,
    /// Exclusion mask half-width in CQT bins (wider than kernel).
    pub mask_half_bins: usize,
    pub q: f32,
    pub damping: f32,
}

#[derive(Debug, Clone)]
pub struct FilterSnapshot {
    pub center_hz: f32,
    pub lane_min_hz: f32,
    pub lane_max_hz: f32,
    pub lane_index: usize,
}

/// Q values for 4th-order Butterworth SOS decomposition.
const BUTTERWORTH_4_Q: [f32; 2] = [0.5412, 1.3066];

impl FilterSnapshot {
    /// Compute the magnitude response of a single RBJ biquad (LP or HP) at
    /// the given frequency.
    fn biquad_lp_response(cutoff_hz: f32, biquad_q: f32, freq_hz: f32, sample_rate: f32) -> f32 {
        let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * biquad_q);
        let cos_w0 = w0.cos();

        // RBJ lowpass coefficients.
        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self::eval_biquad(b0, b1, b2, a0, a1, a2, freq_hz, sample_rate)
    }

    fn biquad_hp_response(cutoff_hz: f32, biquad_q: f32, freq_hz: f32, sample_rate: f32) -> f32 {
        let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * biquad_q);
        let cos_w0 = w0.cos();

        // RBJ highpass coefficients.
        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self::eval_biquad(b0, b1, b2, a0, a1, a2, freq_hz, sample_rate)
    }

    fn eval_biquad(
        b0: f32,
        b1: f32,
        b2: f32,
        a0: f32,
        a1: f32,
        a2: f32,
        freq_hz: f32,
        sample_rate: f32,
    ) -> f32 {
        let w = 2.0 * std::f32::consts::PI * freq_hz / sample_rate;
        let cos_w = w.cos();
        let cos_2w = (2.0 * w).cos();
        let sin_w = w.sin();
        let sin_2w = (2.0 * w).sin();

        let num_re = b0 + b1 * cos_w + b2 * cos_2w;
        let num_im = -(b1 * sin_w + b2 * sin_2w);
        let den_re = a0 + a1 * cos_w + a2 * cos_2w;
        let den_im = -(a1 * sin_w + a2 * sin_2w);

        let num_mag_sq = num_re * num_re + num_im * num_im;
        let den_mag_sq = den_re * den_re + den_im * den_im;

        if den_mag_sq > 1e-20 {
            (num_mag_sq / den_mag_sq).sqrt()
        } else {
            0.0
        }
    }

    /// Compute the magnitude response of the 4th-order Butterworth HP+LP
    /// bandpass at the given frequency. Matches the processor's filter chain.
    pub fn response_at(&self, freq_hz: f32, sample_rate: f32, q: f32) -> f32 {
        // Derive HP and LP cutoffs from center + Q (same formula as processor).
        let half_bw = 1.0 / (2.0 * q);
        let hp_hz = self.center_hz / 2.0_f32.powf(half_bw);
        let lp_hz = self.center_hz * 2.0_f32.powf(half_bw);

        // 4th-order Butterworth = 2 cascaded biquad stages per slope.
        let mut response = 1.0_f32;
        for &bq in &BUTTERWORTH_4_Q {
            response *= Self::biquad_hp_response(hp_hz, bq, freq_hz, sample_rate);
            response *= Self::biquad_lp_response(lp_hz, bq, freq_hz, sample_rate);
        }
        response
    }
}

impl Default for BandSteeringSnapshot {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            score_surface: Vec::new(),
            target_peaks: Vec::new(),
            kernel_half_bins: 0,
            mask_half_bins: 0,
            q: DEFAULT_Q,
            damping: DEFAULT_DAMPING,
        }
    }
}

const DEFAULT_Q: f32 = 2.0; // 0.5 octave bandwidth
const DEFAULT_DAMPING: f32 = 0.9;

/// Exclusion mask is this many times wider than the convolution kernel.
/// Wider mask suppresses side peaks of already-assigned peaks.
const MASK_WIDTH_MULTIPLIER: f32 = 2.0;

/// A new greedy peak must outscore the nearest previous-frame target by this
/// factor to displace it. Prevents targets from ping-ponging between
/// similarly-scored peaks frame to frame.
const STICKY_THRESHOLD: f32 = 1.5;

/// Raised cosine kernel: 0.5 * (1 + cos(π * dist / half_width)).
/// Returns 1.0 at center, 0.0 at ±half_width. Better approximation to
/// bandpass filter response shape than a triangle.
#[inline]
fn cosine_kernel_weight(dist_bins: f32, half_width: f32) -> f32 {
    let d = dist_bins.abs();
    if d >= half_width {
        return 0.0;
    }
    0.5 * (1.0 + (std::f32::consts::PI * d / half_width).cos())
}

/// Scoring mode for the spectral steering metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ScoringMode {
    /// `interest_accum² / magnitude_avg` — rewards dynamics/change.
    InterestQuality = 0,
    /// `magnitude / local_mean` — rewards spectral peaks regardless of dynamics.
    SpectralContrast = 1,
    /// `α * interest_quality + (1-α) * spectral_contrast` — tunable blend.
    Blended = 2,
}

impl ScoringMode {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::SpectralContrast,
            2 => Self::Blended,
            _ => Self::InterestQuality,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::InterestQuality => "Interest*Quality",
            Self::SpectralContrast => "Spectral Contrast",
            Self::Blended => "Blended",
        }
    }

    pub const ALL: [ScoringMode; 3] =
        [Self::InterestQuality, Self::SpectralContrast, Self::Blended];
}

/// Shared tuning parameters — written by GUI, read by spectral thread.
pub struct SharedSteeringParams {
    pub q: AtomicF32Wrapper,
    pub damping: AtomicF32Wrapper,
    pub reset_requested: AtomicU32,
    /// Scoring mode (0=IQ, 1=Contrast, 2=Blended).
    pub scoring_mode: AtomicU32,
    /// Blend alpha for Blended mode (0.0 = pure contrast, 1.0 = pure IQ).
    pub blend_alpha: AtomicF32Wrapper,
}

/// Simple atomic f32 wrapper.
pub struct AtomicF32Wrapper(AtomicU32);

impl AtomicF32Wrapper {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

impl SharedSteeringParams {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            q: AtomicF32Wrapper::new(DEFAULT_Q),
            damping: AtomicF32Wrapper::new(DEFAULT_DAMPING),
            reset_requested: AtomicU32::new(0),
            scoring_mode: AtomicU32::new(ScoringMode::InterestQuality as u32),
            blend_alpha: AtomicF32Wrapper::new(0.5),
        })
    }

    pub fn request_reset(&self) {
        self.reset_requested.store(1, Ordering::Relaxed);
    }

    pub fn take_reset(&self) -> bool {
        self.reset_requested.swap(0, Ordering::Relaxed) != 0
    }
}

impl BandSteering {
    pub fn new() -> Self {
        let lane_configs = vec![
            LaneConfig {
                min_hz: 160.0,
                max_hz: 800.0,
                filter_count: 2,
            },
            LaneConfig {
                min_hz: 800.0,
                max_hz: 4000.0,
                filter_count: 2,
            },
            LaneConfig {
                min_hz: 4000.0,
                max_hz: 20000.0,
                filter_count: 2,
            },
        ];

        let lanes = lane_configs
            .into_iter()
            .map(|config| {
                let filters: Vec<SteerableFilter> = (0..config.filter_count)
                    .map(|i| {
                        let t = (i as f32 + 0.5) / config.filter_count as f32;
                        let center = config.min_hz * (config.max_hz / config.min_hz).powf(t);
                        SteerableFilter {
                            center_hz: center,
                            last_target_bin: None,
                        }
                    })
                    .collect();
                Lane {
                    config,
                    filters,
                    prev_targets: Vec::new(),
                }
            })
            .collect();

        Self {
            lanes,
            q: DEFAULT_Q,
            damping: DEFAULT_DAMPING,
            score_surface: Vec::new(),
            target_peaks: Vec::new(),
            kernel_half_bins: 0,
            mask_half_bins: 0,
        }
    }

    /// Reset all filters to their default positions.
    pub fn reset(&mut self) {
        for lane in &mut self.lanes {
            for (i, filter) in lane.filters.iter_mut().enumerate() {
                let t = (i as f32 + 0.5) / lane.config.filter_count as f32;
                filter.center_hz =
                    lane.config.min_hz * (lane.config.max_hz / lane.config.min_hz).powf(t);
                filter.last_target_bin = None;
            }
            lane.prev_targets.clear();
        }
    }

    /// Sync tuning parameters from the shared atomic state.
    pub fn sync_params(&mut self, params: &SharedSteeringParams) {
        self.q = params.q.get();
        self.damping = params.damping.get();
        if params.take_reset() {
            self.reset();
        }
    }

    /// Update filter positions based on the interest*quality distribution.
    ///
    /// `frequencies` and `interest_quality` are CQT bin frequencies and values.
    /// `_sample_rate` and `_dt` reserved for future use.
    pub fn update(
        &mut self,
        frequencies: &[f32],
        interest_quality: &[f32],
        _sample_rate: f32,
        _dt: f32,
    ) {
        self.score_surface.clear();
        self.target_peaks.clear();

        for (lane_idx, lane) in self.lanes.iter_mut().enumerate() {
            // Find the bin range for this lane.
            let lo_bin = frequencies
                .iter()
                .position(|&f| f >= lane.config.min_hz)
                .unwrap_or(0);
            let hi_bin = frequencies
                .iter()
                .position(|&f| f >= lane.config.max_hz)
                .unwrap_or(frequencies.len());

            if lo_bin >= hi_bin {
                continue;
            }

            let lane_freqs = &frequencies[lo_bin..hi_bin];
            let lane_iq = &interest_quality[lo_bin..hi_bin];
            let lane_bins = lane_freqs.len();

            // Cosine kernel half-width in bins, matched to the HP+LP bandpass width.
            //
            // The bandpass is constructed as HP at center/2^(1/(2Q)) and LP at
            // center*2^(1/(2Q)), giving a total bandwidth of 1/Q octaves.
            // Kernel half-width = full bandwidth in bins.
            let bins_per_octave = crate::spectral::BINS_PER_OCTAVE;
            let q = self.q;
            let bw_octaves = 1.0 / q;
            let kernel_half = ((bw_octaves * bins_per_octave as f32).ceil() as usize).max(1);
            self.kernel_half_bins = kernel_half;
            self.mask_half_bins =
                ((kernel_half as f32 * MASK_WIDTH_MULTIPLIER).ceil() as usize).max(1);

            // Compute score surface: convolve raised cosine kernel with IQ.
            let half_w = kernel_half as f32;
            let mut score_surface = vec![0.0_f32; lane_bins];
            for ci in 0..lane_bins {
                let lo = ci.saturating_sub(kernel_half);
                let hi = (ci + kernel_half + 1).min(lane_bins);
                let mut score = 0.0_f32;
                for bi in lo..hi {
                    let dist = (bi as f32 - ci as f32).abs();
                    let weight = cosine_kernel_weight(dist, half_w);
                    score += lane_iq[bi] * weight;
                }
                score_surface[ci] = score;
            }

            // Store for visualization.
            for (ci, &score) in score_surface.iter().enumerate() {
                self.score_surface.push((lane_freqs[ci], score));
            }

            // Find N target positions by greedy peak-masking with hysteresis.
            //
            // Each greedy pick is compared against previous-frame targets. If
            // a previous target is nearby (within mask distance), the previous
            // position is kept unless the new pick outscores it by
            // STICKY_THRESHOLD. This prevents targets from ping-ponging when
            // two peaks trade being the global maximum frame to frame.
            let n = lane.filters.len();
            let mask_half = half_w * MASK_WIDTH_MULTIPLIER;
            let mut targets = Vec::with_capacity(n);
            let mut masked = score_surface.clone();
            let mut prev_used = vec![false; lane.prev_targets.len()];

            for _ in 0..n {
                let best_bin = masked
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let best_score = masked[best_bin];

                // Check if a previous-frame target is near the new pick.
                // If so, prefer the previous position for stability.
                let chosen_bin = if !lane.prev_targets.is_empty() {
                    let mut nearest_prev = None;
                    let mut nearest_dist = f32::MAX;
                    for (pi, &prev_bin) in lane.prev_targets.iter().enumerate() {
                        if prev_used[pi] || prev_bin >= lane_bins {
                            continue;
                        }
                        let dist = (best_bin as f32 - prev_bin as f32).abs();
                        if dist < mask_half && dist < nearest_dist {
                            nearest_dist = dist;
                            nearest_prev = Some(pi);
                        }
                    }
                    if let Some(pi) = nearest_prev {
                        let prev_bin = lane.prev_targets[pi];
                        let prev_score = score_surface[prev_bin];
                        prev_used[pi] = true;
                        // Keep previous position unless new one is much better.
                        if best_score > prev_score * STICKY_THRESHOLD {
                            best_bin
                        } else {
                            prev_bin
                        }
                    } else {
                        best_bin
                    }
                } else {
                    best_bin
                };

                targets.push(chosen_bin);

                // Mask out a wider region to suppress side peaks.
                for bi in 0..masked.len() {
                    let dist = (bi as f32 - chosen_bin as f32).abs();
                    let weight = cosine_kernel_weight(dist, mask_half);
                    if weight > 0.0 {
                        masked[bi] *= 1.0 - weight;
                    }
                }
            }

            // Save targets for next frame's hysteresis.
            lane.prev_targets = targets.clone();

            // Store target peaks for visualization.
            for &bin in &targets {
                self.target_peaks.push((lane_freqs[bin], lane_idx));
            }

            // Stable assignment: match each target to its nearest filter,
            // not each filter to its nearest target. This prevents filters
            // from swapping peaks when magnitudes trade off — each peak
            // "claims" the filter closest to it.
            //
            // For filters with history, we match based on last_target_bin
            // (where the filter was pointing). For filters without history,
            // we match based on current position.
            let mut filter_assigned = vec![false; n];
            let mut target_for_filter: Vec<Option<usize>> = vec![None; n];

            // First pass: for each target (in score order, strongest first),
            // find the nearest unassigned filter and claim it.
            for (ti, &target_bin) in targets.iter().enumerate() {
                let mut best_fi = None;
                let mut best_dist = f32::MAX;
                for (fi, filter) in lane.filters.iter().enumerate() {
                    if filter_assigned[fi] {
                        continue;
                    }
                    let filter_bin = filter.last_target_bin.unwrap_or_else(|| {
                        lane_freqs
                            .iter()
                            .position(|&f| f >= filter.center_hz)
                            .unwrap_or(0)
                    });
                    let dist = (filter_bin as f32 - target_bin as f32).abs();
                    if dist < best_dist {
                        best_dist = dist;
                        best_fi = Some(fi);
                    }
                }
                if let Some(fi) = best_fi {
                    filter_assigned[fi] = true;
                    target_for_filter[fi] = Some(ti);
                }
            }

            // Apply assignments.
            for (fi, filter) in lane.filters.iter_mut().enumerate() {
                let ti = target_for_filter[fi].unwrap_or(0);
                filter.last_target_bin = Some(targets[ti]);

                let target_hz = lane_freqs[targets[ti]];
                let log_current = filter.center_hz.max(1.0).ln();
                let log_target = target_hz.max(1.0).ln();
                let new_log = self.damping * log_current + (1.0 - self.damping) * log_target;
                filter.center_hz = new_log.exp().clamp(lane.config.min_hz, lane.config.max_hz);
            }
        }
    }

    /// Write current filter positions and Q to the shared atomic state
    /// so the audio callback can read them.
    pub fn write_shared_freqs(&self, shared: &SharedFilterFreqs) {
        let mut idx = 0;
        for lane in &self.lanes {
            for filter in &lane.filters {
                if idx < STEERED_FILTER_COUNT {
                    shared.center_hz[idx].set(filter.center_hz);
                    idx += 1;
                }
            }
        }
        shared.q.set(self.q);
    }

    /// Produce a snapshot of current filter positions for the GUI.
    /// Filters within each lane are sorted by frequency so that the
    /// flat index order matches the envelope output order.
    pub fn snapshot(&self) -> BandSteeringSnapshot {
        let mut filters = Vec::new();
        for (lane_idx, lane) in self.lanes.iter().enumerate() {
            let mut lane_filters: Vec<FilterSnapshot> = lane
                .filters
                .iter()
                .map(|filter| FilterSnapshot {
                    center_hz: filter.center_hz,
                    lane_min_hz: lane.config.min_hz,
                    lane_max_hz: lane.config.max_hz,
                    lane_index: lane_idx,
                })
                .collect();
            lane_filters.sort_by(|a, b| a.center_hz.partial_cmp(&b.center_hz).unwrap());
            filters.extend(lane_filters);
        }
        BandSteeringSnapshot {
            filters,
            score_surface: self.score_surface.clone(),
            target_peaks: self.target_peaks.clone(),
            kernel_half_bins: self.kernel_half_bins,
            mask_half_bins: self.mask_half_bins,
            q: self.q,
            damping: self.damping,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_initialize_within_lanes() {
        let steering = BandSteering::new();
        for lane in &steering.lanes {
            for filter in &lane.filters {
                assert!(
                    filter.center_hz >= lane.config.min_hz
                        && filter.center_hz <= lane.config.max_hz,
                    "Filter at {:.0} Hz outside lane [{:.0}, {:.0}]",
                    filter.center_hz,
                    lane.config.min_hz,
                    lane.config.max_hz,
                );
            }
        }
    }

    #[test]
    fn filters_stay_in_lane() {
        let mut steering = BandSteering::new();

        // Put all interest below the lowest lane. Filters should not escape.
        let num_bins = 400;
        let frequencies: Vec<f32> = (0..num_bins)
            .map(|i| 160.0 * (20000.0_f32 / 160.0).powf(i as f32 / num_bins as f32))
            .collect();
        let mut interest_quality = vec![0.0_f32; num_bins];
        // Put interest at the very bottom.
        interest_quality[0] = 10.0;
        interest_quality[1] = 10.0;

        steering.update(&frequencies, &interest_quality, 48000.0, 0.085);

        for lane in &steering.lanes {
            for filter in &lane.filters {
                assert!(
                    filter.center_hz >= lane.config.min_hz,
                    "Filter at {:.0} Hz escaped lane min {:.0}",
                    filter.center_hz,
                    lane.config.min_hz,
                );
            }
        }
    }
}
