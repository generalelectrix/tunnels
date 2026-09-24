//! IIR allpass-pair Hilbert transformer for computing the analytic signal.
//!
//! Based on Olli Niemitalo's design using cascaded allpass sections.
//! Each path is a chain of 4 second-order allpass sections with different
//! coefficients, producing two outputs with ~90-degree phase difference
//! across nearly the full bandwidth.
//!
//! The magnitude of the analytic signal `sqrt(path0^2 + path1^2)` gives
//! the instantaneous amplitude envelope without rectification harmonics.
//!
//! The two chains are in phase with each other; the quadrature comes from
//! delaying one of them by a sample, which is what turns the phase
//! difference between them into the 90 degrees the magnitude needs.

/// Coefficients for the two allpass chains.
/// From Olli Niemitalo's design, optimized for +/-0.7 degree accuracy
/// over 99.8% of the bandwidth.
const COEFFS_PATH0: [f64; 4] = [0.6923878, 0.9360654322959, 0.9882295226860, 0.9987488452737];

const COEFFS_PATH1: [f64; 4] = [
    0.4021921162426,
    0.8561710882420,
    0.9722909545651,
    0.9952884791278,
];

/// A single second-order allpass section.
/// Transfer function: H(z) = (a^2 - z^-2) / (1 - a^2 * z^-2)
/// Difference equation: out[n] = a^2 * (in[n] + out[n-2]) - in[n-2]
#[derive(Clone)]
struct AllpassSection {
    a_squared: f64,
    /// Inputs and outputs from the two previous samples, newest first.
    x_prev: [f64; 2],
    y_prev: [f64; 2],
}

impl AllpassSection {
    fn new(a: f64) -> Self {
        Self {
            a_squared: a * a,
            x_prev: [0.0; 2],
            y_prev: [0.0; 2],
        }
    }

    #[inline]
    fn process(&mut self, input: f64) -> f64 {
        let output = self.a_squared * (input + self.y_prev[1]) - self.x_prev[1];
        self.x_prev = [input, self.x_prev[0]];
        self.y_prev = [output, self.y_prev[0]];
        output
    }
}

/// IIR Hilbert transformer producing two quadrature outputs.
#[derive(Clone)]
pub struct HilbertTransform {
    path0: [AllpassSection; 4],
    path1: [AllpassSection; 4],
    /// Path 0's previous output: the one-sample delay that puts the two
    /// paths in quadrature.
    path0_delay: f64,
}

impl Default for HilbertTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl HilbertTransform {
    pub fn new() -> Self {
        Self {
            path0: [
                AllpassSection::new(COEFFS_PATH0[0]),
                AllpassSection::new(COEFFS_PATH0[1]),
                AllpassSection::new(COEFFS_PATH0[2]),
                AllpassSection::new(COEFFS_PATH0[3]),
            ],
            path1: [
                AllpassSection::new(COEFFS_PATH1[0]),
                AllpassSection::new(COEFFS_PATH1[1]),
                AllpassSection::new(COEFFS_PATH1[2]),
                AllpassSection::new(COEFFS_PATH1[3]),
            ],
            path0_delay: 0.0,
        }
    }

    /// Process one sample, returning the instantaneous amplitude (envelope).
    #[inline]
    pub fn envelope(&mut self, input: f64) -> f64 {
        let mut out0 = input;
        for section in &mut self.path0 {
            out0 = section.process(out0);
        }
        let mut out1 = input;
        for section in &mut self.path1 {
            out1 = section.process(out1);
        }
        let delayed0 = std::mem::replace(&mut self.path0_delay, out0);
        (delayed0 * delayed0 + out1 * out1).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// The envelope of a constant sine is its amplitude, at any frequency a
    /// band carries. A quadrature pair holds across the whole bandwidth, so
    /// one frequency proves very little on its own.
    #[test]
    fn sine_envelope_is_approximately_constant() {
        let sample_rate = 48000.0;
        let amplitude = 0.8;

        for freq in [50.0, 440.0, 1000.0, 3000.0, 6000.0, 10000.0] {
            let mut hilbert = HilbertTransform::new();
            // Run for a bit to let the filters settle.
            for i in 0..4800 {
                let t = i as f64 / sample_rate;
                hilbert.envelope(amplitude * (2.0 * PI * freq * t).sin());
            }

            let mut min_env = f64::MAX;
            let mut max_env = f64::MIN;
            for i in 4800..9600 {
                let t = i as f64 / sample_rate;
                let env = hilbert.envelope(amplitude * (2.0 * PI * freq * t).sin());
                min_env = min_env.min(env);
                max_env = max_env.max(env);
            }

            let ripple = max_env - min_env;
            assert!(
                ripple < 0.05,
                "envelope ripple {ripple:.4} too large for a {freq} Hz sine"
            );
            let mean = (min_env + max_env) / 2.0;
            assert!(
                (mean - amplitude).abs() < 0.05,
                "envelope mean {mean:.4} should be close to amplitude {amplitude} at {freq} Hz"
            );
        }
    }

    #[test]
    fn silence_produces_zero() {
        let mut hilbert = HilbertTransform::new();
        for _ in 0..1000 {
            let env = hilbert.envelope(0.0);
            assert!(env.abs() < 1e-10);
        }
    }
}
