//! Adaptive band steering: a committee of narrow bandpass filters that
//! drift toward regions of spectral interest.
//!
//! Each filter has a center frequency constrained to a lane (frequency range).
//! Within its lane, the filter is attracted toward interest*quality mass and
//! repelled from other filters in the same lane via Gaussian repulsion.
//!
//! The filter positions are updated at the spectral analysis rate (~12 Hz).
//! No actual audio filtering happens here — this module only computes where
//! the filters *should* be. The audio thread reads the positions and applies
//! bandpass filters separately.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Shared tuning parameters for band steering — written by GUI, read by spectral thread.
pub struct SharedSteeringParams {
    pub q: AtomicF32Wrapper,
    pub damping: AtomicF32Wrapper,
    pub reset_requested: AtomicU32,
}

/// Simple atomic f32 wrapper (we can't use audio_processor_traits::AtomicF32 here
/// without adding that dep, so roll a minimal one).
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
        })
    }

    pub fn request_reset(&self) {
        self.reset_requested.store(1, Ordering::Relaxed);
    }

    pub fn take_reset(&self) -> bool {
        self.reset_requested.swap(0, Ordering::Relaxed) != 0
    }
}

/// Configuration for a lane — a frequency region containing one or more steerable filters.
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
}

/// A lane containing one or more steerable filters.
#[derive(Debug, Clone)]
pub struct Lane {
    pub config: LaneConfig,
    pub filters: Vec<SteerableFilter>,
}

/// The complete band steering state.
pub struct BandSteering {
    pub lanes: Vec<Lane>,
    /// Q factor for the bandpass filters (higher = narrower).
    pub q: f32,
    /// Smoothing factor for filter movement (0 = instant, 1 = frozen).
    pub damping: f32,
    /// Score surface from the most recent update.
    score_surface: Vec<(f32, f32)>,
}

/// Snapshot of filter positions and tuning parameters for the GUI.
#[derive(Debug, Clone)]
pub struct BandSteeringSnapshot {
    pub filters: Vec<FilterSnapshot>,
    /// Score surface: (frequency, score) pairs for visualization.
    /// The convolution of the filter response with the IQ distribution.
    pub score_surface: Vec<(f32, f32)>,
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

impl FilterSnapshot {
    /// Compute the magnitude response of an RBJ bandpass filter at the given
    /// frequency. Returns a value in [0, 1] where 1 = full passthrough.
    pub fn response_at(&self, freq_hz: f32, sample_rate: f32, q: f32) -> f32 {
        let w0 = 2.0 * std::f32::consts::PI * self.center_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);

        let b0 = alpha;
        let b1 = 0.0_f32;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * w0.cos();
        let a2 = 1.0 - alpha;

        // Evaluate |H(e^{jω})| where ω = 2π·freq/sr.
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
}

impl Default for BandSteeringSnapshot {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            score_surface: Vec::new(),
            q: DEFAULT_Q,
            damping: DEFAULT_DAMPING,
        }
    }
}

// Defaults tuned for log-frequency dynamics where distances are
// in units of ln(Hz) — one octave ≈ 0.69.
const DEFAULT_DAMPING: f32 = 0.9;
const DEFAULT_Q: f32 = 4.0;


impl BandSteering {
    /// Create a new band steering system with the default lane configuration.
    pub fn new() -> Self {
        let lane_configs = vec![
            LaneConfig {
                min_hz: 20.0,
                max_hz: 150.0,
                filter_count: 1,
            },
            LaneConfig {
                min_hz: 150.0,
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
                filter_count: 3,
            },
        ];

        let lanes = lane_configs
            .into_iter()
            .map(|config| {
                let filters: Vec<SteerableFilter> = (0..config.filter_count)
                    .map(|i| {
                        // Initialize evenly spaced within the lane (log-spaced).
                        let t = (i as f32 + 0.5) / config.filter_count as f32;
                        let center = config.min_hz
                            * (config.max_hz / config.min_hz).powf(t);
                        SteerableFilter { center_hz: center }
                    })
                    .collect();
                Lane { config, filters }
            })
            .collect();

        Self {
            lanes,
            q: DEFAULT_Q,
            damping: DEFAULT_DAMPING,
            score_surface: Vec::new(),
        }
    }

    /// Update all filter positions based on the interest*quality distribution.
    ///
    /// `frequencies` and `interest_quality` must be the same length — the per-bin
    /// frequency and interest*quality values from the spectral snapshot.
    /// Reset all filters to their default positions.
    pub fn reset(&mut self) {
        for lane in &mut self.lanes {
            for (i, filter) in lane.filters.iter_mut().enumerate() {
                let t = (i as f32 + 0.5) / lane.config.filter_count as f32;
                filter.center_hz =
                    lane.config.min_hz * (lane.config.max_hz / lane.config.min_hz).powf(t);
            }
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

    pub fn update(&mut self, frequencies: &[f32], interest_quality: &[f32], sample_rate: f32) {
        self.score_surface.clear();
        for lane in &mut self.lanes {
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

            // Compute the "score surface": for each possible center frequency,
            // how much interest*quality would a bandpass filter capture there?
            // This is the dot product of the filter response with the IQ distribution.
            let mut score_surface = vec![0.0_f32; lane_bins];
            for (ci, &candidate_freq) in lane_freqs.iter().enumerate() {
                let snap = FilterSnapshot {
                    center_hz: candidate_freq,
                    lane_min_hz: lane.config.min_hz,
                    lane_max_hz: lane.config.max_hz,
                    lane_index: 0,
                };
                let mut score = 0.0_f32;
                for (&freq, &iq) in lane_freqs.iter().zip(lane_iq) {
                    if iq > 0.0 {
                        let response = snap.response_at(freq, sample_rate, self.q);
                        score += iq * response;
                    }
                }
                score_surface[ci] = score;
            }

            // Store the score surface for visualization.
            for (ci, &score) in score_surface.iter().enumerate() {
                self.score_surface.push((lane_freqs[ci], score));
            }

            // Find N target positions by greedy peak-masking.
            let n = lane.filters.len();
            let mut targets = Vec::with_capacity(n);
            let mut masked = score_surface;

            for _ in 0..n {
                let best_bin = masked
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                    .map(|(i, _)| i)
                    .unwrap_or(0);

                let target_hz = lane_freqs[best_bin];
                targets.push(target_hz);

                // Mask out the region around this peak.
                let mask_snap = FilterSnapshot {
                    center_hz: target_hz,
                    lane_min_hz: lane.config.min_hz,
                    lane_max_hz: lane.config.max_hz,
                    lane_index: 0,
                };
                for (bi, &freq) in lane_freqs.iter().enumerate() {
                    let response = mask_snap.response_at(freq, sample_rate, self.q);
                    masked[bi] *= 1.0 - response;
                }
            }

            // Assign each filter to the nearest unassigned target (in log space,
            // so "nearest" means nearest in octaves).
            let log_targets: Vec<f32> = targets.iter().map(|&f| f.max(1.0).ln()).collect();
            let mut assigned = vec![false; n];
            for filter in &mut lane.filters {
                let log_center = filter.center_hz.max(1.0).ln();
                let mut best_target = 0;
                let mut best_dist = f32::MAX;
                for (ti, &log_target) in log_targets.iter().enumerate() {
                    if assigned[ti] {
                        continue;
                    }
                    let dist = (log_center - log_target).abs();
                    if dist < best_dist {
                        best_dist = dist;
                        best_target = ti;
                    }
                }
                assigned[best_target] = true;

                // Smooth toward the assigned target in log space.
                let log_target = log_targets[best_target];
                let new_log = self.damping * log_center + (1.0 - self.damping) * log_target;
                filter.center_hz = new_log.exp().clamp(
                    lane.config.min_hz,
                    lane.config.max_hz,
                );
            }
        }
    }

    /// Produce a snapshot of current filter positions for the GUI.
    pub fn snapshot(&self) -> BandSteeringSnapshot {
        let mut filters = Vec::new();
        for (lane_idx, lane) in self.lanes.iter().enumerate() {
            for filter in &lane.filters {
                filters.push(FilterSnapshot {
                    center_hz: filter.center_hz,
                    lane_min_hz: lane.config.min_hz,
                    lane_max_hz: lane.config.max_hz,
                    lane_index: lane_idx,
                });
            }
        }
        BandSteeringSnapshot {
            filters,
            score_surface: self.score_surface.clone(),
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

        // Put all interest at 50 Hz (sub-bass). The 150-800 Hz lane filters
        // should NOT drift below 150 Hz.
        let bin_width = 3.0;
        let num_bins = 8000;
        let frequencies: Vec<f32> = (0..num_bins).map(|i| i as f32 * bin_width).collect();
        let mut interest_quality = vec![0.0_f32; num_bins];
        let peak_bin = (50.0 / bin_width) as usize;
        for i in peak_bin.saturating_sub(3)..=(peak_bin + 3).min(num_bins - 1) {
            interest_quality[i] = 10.0;
        }

        // One update is enough — the convolution directly computes optimal positions.
        steering.update(&frequencies, &interest_quality, 48000.0);

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
