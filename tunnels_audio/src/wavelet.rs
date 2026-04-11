//! Streaming discrete wavelet transform (DWT) using Daubechies filters.
//!
//! Decomposes audio into octave bands in real-time. Each level splits the
//! signal into a low and high subband via halfband FIR filters with
//! decimation by 2. The high subband at each level is one octave-wide band.
//! The final low subband captures everything below the lowest split.
//!
//! At 48kHz with 7 levels:
//!   Level 1: 12-24 kHz (high), running at 24kHz
//!   Level 2: 6-12 kHz (high), running at 12kHz
//!   Level 3: 3-6 kHz (high), running at 6kHz
//!   Level 4: 1.5-3 kHz (high), running at 3kHz
//!   Level 5: 750-1500 Hz (high), running at 1.5kHz
//!   Level 6: 375-750 Hz (high), running at 750Hz
//!   Level 7: 187-375 Hz (high), running at 375Hz
//!   Residual: 0-187 Hz (low), running at 375Hz

/// Daubechies-4 (db4) filter coefficients — 8 taps.
/// ~18 dB/octave transition steepness.
const DB4_LO: [f32; 8] = [
    -0.010_597_402,
    0.032_883_01,
    0.030_841_382,
    -0.187_034_82,
    -0.027_983_77,
    0.630_880_8,
    0.714_846_55,
    0.230_377_81,
];

/// Daubechies-8 (db8) filter coefficients — 16 taps.
/// ~30 dB/octave transition steepness.
const DB8_LO: [f32; 16] = [
    -0.000_117_476_78,
    0.000_675_449_4,
    -0.000_391_740_38,
    -0.004_870_353,
    0.008_746_094,
    0.013_981_027_5,
    -0.044_088_256,
    -0.017_369_3,
    0.128_747_43,
    0.000_472_484_56,
    -0.284_015_54,
    -0.015_829_105,
    0.585_354_7,
    0.675_630_75,
    0.312_871_6,
    0.054_415_84,
];

/// Derive the highpass filter from the lowpass using the QMF relation:
/// h[n] = (-1)^n * g[N-1-n]
fn qmf_highpass(lowpass: &[f32]) -> Vec<f32> {
    let n = lowpass.len();
    (0..n)
        .map(|i| {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            sign * lowpass[n - 1 - i]
        })
        .collect()
}

/// Which Daubechies wavelet to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveletType {
    /// 8 taps, ~18 dB/oct, ~1.3ms latency at level 4
    Daubechies4,
    /// 16 taps, ~30 dB/oct, ~2.5ms latency at level 4
    Daubechies8,
}

/// A single decomposition level: lowpass + highpass FIR with decimation by 2.
struct Level {
    /// Lowpass FIR coefficients.
    lo_coeffs: Vec<f32>,
    /// Highpass FIR coefficients.
    hi_coeffs: Vec<f32>,
    /// Delay line for input samples.
    delay: Vec<f32>,
    /// Write position into the delay line.
    delay_pos: usize,
    /// Decimation phase: process output every 2nd input sample.
    phase: bool,
}

impl Level {
    fn new(lo_coeffs: &[f32]) -> Self {
        let hi_coeffs = qmf_highpass(lo_coeffs);
        let taps = lo_coeffs.len();
        Self {
            lo_coeffs: lo_coeffs.to_vec(),
            hi_coeffs,
            delay: vec![0.0; taps],
            delay_pos: 0,
            phase: false,
        }
    }

    /// Push one input sample. Returns Some((low, high)) every 2nd sample
    /// (decimation by 2), or None on the off-phase.
    #[inline]
    fn push(&mut self, sample: f32) -> Option<(f32, f32)> {
        let taps = self.lo_coeffs.len();

        // Write into circular delay line.
        self.delay[self.delay_pos] = sample;
        self.delay_pos = (self.delay_pos + 1) % taps;

        // Only compute output every other sample.
        self.phase = !self.phase;
        if !self.phase {
            return None;
        }

        // Convolve with both filters.
        let mut lo = 0.0_f32;
        let mut hi = 0.0_f32;
        for k in 0..taps {
            let idx = (self.delay_pos + taps - 1 - k) % taps;
            let s = self.delay[idx];
            lo += s * self.lo_coeffs[k];
            hi += s * self.hi_coeffs[k];
        }

        Some((lo, hi))
    }

    /// Reset the delay line to zero.
    fn reset(&mut self) {
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.phase = false;
    }
}

/// Number of octave decomposition levels.
/// At 48kHz this gives bands down to ~187 Hz.
pub const NUM_LEVELS: usize = 7;

/// Total number of output bands: NUM_LEVELS high bands + 1 residual low band.
pub const NUM_BANDS: usize = NUM_LEVELS + 1;

/// Band labels in frequency-ascending order, matching the output band indices
/// used by the processor (index 0 = lowpass sub-bass, 7 = highest wavelet band).
/// Valid for 48kHz sample rate with NUM_LEVELS = 7.
pub const BAND_LABELS: [&str; NUM_BANDS] = [
    "Lowpass", "187-375", "375-750", "750-1.5k", "1.5-3k", "3-6k", "6-12k", "12-24k",
];

/// Streaming wavelet decomposition.
///
/// Push one audio sample. A callback receives `(band_index, sample)` for
/// each band that produces output on this call. Higher bands fire every
/// call; lower bands fire less frequently due to decimation.
pub struct WaveletDecomposition {
    levels: Vec<Level>,
}

impl WaveletDecomposition {
    pub fn new(wavelet: WaveletType) -> Self {
        let coeffs: &[f32] = match wavelet {
            WaveletType::Daubechies4 => &DB4_LO,
            WaveletType::Daubechies8 => &DB8_LO,
        };
        let levels = (0..NUM_LEVELS).map(|_| Level::new(coeffs)).collect();
        Self { levels }
    }

    /// Change the wavelet type. Resets all filter state.
    pub fn set_wavelet(&mut self, wavelet: WaveletType) {
        let coeffs: &[f32] = match wavelet {
            WaveletType::Daubechies4 => &DB4_LO,
            WaveletType::Daubechies8 => &DB8_LO,
        };
        for level in &mut self.levels {
            level.lo_coeffs = coeffs.to_vec();
            level.hi_coeffs = qmf_highpass(coeffs);
            level.delay.resize(coeffs.len(), 0.0);
            level.reset();
        }
    }

    /// Process one input sample through the decomposition tree.
    /// Calls `on_band(band_index, sample)` for each band that produces output.
    /// Band 0 = highest frequency (12-24kHz), band NUM_LEVELS = residual low.
    #[inline]
    pub fn push(&mut self, sample: f32, mut on_band: impl FnMut(usize, f32)) {
        let mut current = sample;

        for level_idx in 0..NUM_LEVELS {
            match self.levels[level_idx].push(current) {
                Some((lo, hi)) => {
                    on_band(level_idx, hi);
                    current = lo;
                }
                None => {
                    return;
                }
            }
        }

        // The residual low subband from the deepest level.
        on_band(NUM_LEVELS, current);
    }

    /// Reset all filter state.
    pub fn reset(&mut self) {
        for level in &mut self.levels {
            level.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposition_produces_output() {
        let mut dwt = WaveletDecomposition::new(WaveletType::Daubechies4);
        let mut band_energy = [0.0_f32; NUM_BANDS];

        let sample_rate = 48000.0_f32;
        let freq = 1000.0; // Should land in 750-1500 Hz band (index 4).

        for i in 0..4800 {
            let t = i as f32 / sample_rate;
            let sample = (2.0 * std::f32::consts::PI * freq * t).sin();
            dwt.push(sample, |band, s| {
                band_energy[band] = band_energy[band].max(s.abs());
            });
        }

        let target_band = 4;
        assert!(
            band_energy[target_band] > 0.01,
            "Expected energy in band {target_band}, got {}",
            band_energy[target_band]
        );
    }

    #[test]
    fn silence_produces_zero() {
        let mut dwt = WaveletDecomposition::new(WaveletType::Daubechies4);
        let mut any_nonzero = false;
        for _ in 0..4800 {
            dwt.push(0.0, |_band, s| {
                if s.abs() > 1e-10 {
                    any_nonzero = true;
                }
            });
        }
        assert!(!any_nonzero, "Expected silence in all bands");
    }

    #[test]
    fn both_wavelet_types_work() {
        for wtype in [WaveletType::Daubechies4, WaveletType::Daubechies8] {
            let mut dwt = WaveletDecomposition::new(wtype);
            let mut max_energy = 0.0_f32;
            let sample_rate = 48000.0_f32;
            for i in 0..4800 {
                let t = i as f32 / sample_rate;
                let sample = (2.0 * std::f32::consts::PI * 2000.0 * t).sin();
                dwt.push(sample, |_band, s| {
                    max_energy = max_energy.max(s.abs());
                });
            }
            assert!(max_energy > 0.01, "No energy detected with {wtype:?}");
        }
    }

    #[test]
    fn reset_clears_state() {
        let mut dwt = WaveletDecomposition::new(WaveletType::Daubechies4);
        for i in 0..4800 {
            dwt.push((i as f32 * 0.1).sin(), |_, _| {});
        }
        dwt.reset();
        let mut any_nonzero = false;
        // After reset, feeding silence should produce exactly zero.
        for _ in 0..480 {
            dwt.push(0.0, |_band, s| {
                if s.abs() > 1e-10 {
                    any_nonzero = true;
                }
            });
        }
        assert!(!any_nonzero, "Energy should be zero after reset + silence");
    }
}
