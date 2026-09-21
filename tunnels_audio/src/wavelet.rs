//! Streaming stationary wavelet transform (SWT) using Daubechies filters.
//!
//! Decomposes audio into octave bands in real time. Each level splits its
//! input into a low and high subband with a pair of halfband FIR filters;
//! the high subband is one octave-wide band and the low subband feeds the
//! next level. Nothing is decimated: level `k` instead dilates its filters
//! by `2^k` (the "à trous" construction), so every band is produced at the
//! full sample rate and the transform is shift-invariant. A subband here is
//! exactly the decimated transform's subband computed at every phase at
//! once, with nothing folded in from above the band.
//!
//! At 48kHz with 7 levels:
//!   Level 1: 12-24 kHz (high)
//!   Level 2: 6-12 kHz (high)
//!   Level 3: 3-6 kHz (high)
//!   Level 4: 1.5-3 kHz (high)
//!   Level 5: 750-1500 Hz (high)
//!   Level 6: 375-750 Hz (high)
//!   Level 7: 187-375 Hz (high)
//!   Residual: 0-187 Hz (low)

/// Daubechies-4 (db4) lowpass decomposition filter — 8 taps, ~18 dB/octave
/// transition steepness — in its conventional orientation, with the large
/// taps last.
const DB4_DEC_LO: [f32; 8] = [
    -0.010_597_402,
    0.032_883_01,
    0.030_841_382,
    -0.187_034_82,
    -0.027_983_77,
    0.630_880_8,
    0.714_846_55,
    0.230_377_81,
];

/// The orthonormal taps have a passband gain of √2 per level; scaled by
/// this, every band has unit passband gain, so a level means the same in
/// each and the residual is not 2^(levels/2) louder than its input.
const UNIT_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The lowpass taps as applied, newest sample first. Daubechies filters are
/// not linear phase, so orientation sets the group delay: with the large
/// taps on the newest samples each level delays by about one stride, with
/// them on the oldest by about six. Nothing is reconstructed from these
/// bands, so the low-delay orientation costs nothing; magnitude response is
/// identical either way.
const DB4_LO: [f32; 8] = scaled(reversed(DB4_DEC_LO), UNIT_GAIN);

/// The highpass taps, from the QMF relation `h[n] = (-1)^n g[N-1-n]` on the
/// conventional lowpass, which already puts the large taps first.
const DB4_HI: [f32; 8] = scaled(qmf_highpass(DB4_DEC_LO), UNIT_GAIN);

const fn scaled<const N: usize>(taps: [f32; N], by: f32) -> [f32; N] {
    let mut out = [0.0; N];
    let mut i = 0;
    while i < N {
        out[i] = taps[i] * by;
        i += 1;
    }
    out
}

const fn reversed<const N: usize>(taps: [f32; N]) -> [f32; N] {
    let mut out = [0.0; N];
    let mut i = 0;
    while i < N {
        out[i] = taps[N - 1 - i];
        i += 1;
    }
    out
}

const fn qmf_highpass<const N: usize>(lowpass: [f32; N]) -> [f32; N] {
    let mut hi = [0.0; N];
    let mut i = 0;
    while i < N {
        hi[i] = if i % 2 == 0 { 1.0 } else { -1.0 } * lowpass[N - 1 - i];
        i += 1;
    }
    hi
}

/// A single decomposition level: lowpass + highpass FIR, dilated by the
/// level's stride, producing an output for every input sample.
struct Level {
    /// Lowpass FIR taps.
    lo: [f32; 8],
    /// Highpass FIR taps.
    hi: [f32; 8],
    /// Spacing between taps in samples: `2^level`.
    stride: usize,
    /// Delay line for input samples, `taps * stride` long — a power of two,
    /// so positions wrap by masking.
    delay: Vec<f32>,
    /// Write position into the delay line.
    delay_pos: usize,
}

impl Level {
    fn new(stride: usize) -> Self {
        let len = DB4_LO.len() * stride;
        debug_assert!(len.is_power_of_two());
        Self {
            lo: DB4_LO,
            hi: DB4_HI,
            stride,
            delay: vec![0.0; len],
            delay_pos: 0,
        }
    }

    /// Push one input sample and return the (low, high) subband outputs.
    #[inline]
    fn push(&mut self, sample: f32) -> (f32, f32) {
        let mask = self.delay.len() - 1;

        self.delay[self.delay_pos] = sample;
        self.delay_pos = (self.delay_pos + 1) & mask;

        // Convolve with both filters, taps `stride` samples apart, newest
        // sample first.
        let mut lo = 0.0_f32;
        let mut hi = 0.0_f32;
        let mut idx = self.delay_pos.wrapping_sub(1) & mask;
        for (l, h) in self.lo.iter().zip(&self.hi) {
            let s = self.delay[idx];
            lo += s * l;
            hi += s * h;
            idx = idx.wrapping_sub(self.stride) & mask;
        }

        (lo, hi)
    }
}

/// Number of octave decomposition levels.
/// At 48kHz this gives bands down to ~187 Hz.
pub const NUM_LEVELS: usize = 7;

/// Total number of output bands: NUM_LEVELS high bands + 1 residual low band.
pub const NUM_BANDS: usize = NUM_LEVELS + 1;

/// Streaming wavelet decomposition.
///
/// Push one audio sample; a callback receives every level's output on every call.
pub struct WaveletDecomposition {
    levels: Vec<Level>,
}

impl WaveletDecomposition {
    pub fn new() -> Self {
        let levels = (0..NUM_LEVELS)
            .map(|level| Level::new(1 << level))
            .collect();
        Self { levels }
    }

    /// Process one input sample through the decomposition tree.
    /// Calls `on_band(level, sample)` for every level: level 0 is the
    /// highest octave, level `NUM_LEVELS` the residual below the lowest.
    #[inline]
    pub fn push(&mut self, sample: f32, mut on_band: impl FnMut(usize, f32)) {
        let mut current = sample;

        for (level_idx, level) in self.levels.iter_mut().enumerate() {
            let (lo, hi) = level.push(current);
            on_band(level_idx, hi);
            current = lo;
        }

        // The residual low subband from the deepest level.
        on_band(NUM_LEVELS, current);
    }
}

impl Default for WaveletDecomposition {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposition_produces_output() {
        let mut dwt = WaveletDecomposition::new();
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
        let mut dwt = WaveletDecomposition::new();
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
}
