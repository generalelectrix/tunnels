//! IIR allpass-pair Hilbert transformer for computing the analytic signal.
//!
//! Based on Olli Niemitalo's design using cascaded allpass sections.
//! Each path is a chain of 4 second-order allpass sections with different
//! coefficients, producing two outputs with ~90-degree phase difference
//! across nearly the full bandwidth.
//!
//! The magnitude of the analytic signal `sqrt(path0^2 + path1^2)` gives
//! the instantaneous amplitude envelope without rectification harmonics.

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
/// Transfer function: H(z) = (a^2 + z^-2) / (1 + a^2 * z^-2)
/// Difference equation: out[n] = a^2 * (in[n] + out[n-2]) - in[n-2]
#[derive(Clone)]
struct AllpassSection {
    a_squared: f64,
    x_prev2: f64, // in[n-2]
    y_prev2: f64, // out[n-2]
}

impl AllpassSection {
    fn new(a: f64) -> Self {
        Self {
            a_squared: a * a,
            x_prev2: 0.0,
            y_prev2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f64) -> f64 {
        let output = self.a_squared * (input + self.y_prev2) - self.x_prev2;
        self.x_prev2 = input;
        self.y_prev2 = output;
        output
    }
}

/// IIR Hilbert transformer producing two quadrature outputs.
#[derive(Clone)]
pub struct HilbertTransform {
    path0: [AllpassSection; 4],
    path1: [AllpassSection; 4],
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
        (out0 * out0 + out1 * out1).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn sine_envelope_is_approximately_constant() {
        let mut hilbert = HilbertTransform::new();
        let sample_rate = 48000.0;
        let freq = 440.0;
        let amplitude = 0.8;

        // Run for a bit to let the filters settle.
        for i in 0..4800 {
            let t = i as f64 / sample_rate;
            let sample = amplitude * (2.0 * PI * freq * t).sin();
            hilbert.envelope(sample);
        }

        // Now check that the envelope is close to the amplitude.
        let mut min_env = f64::MAX;
        let mut max_env = f64::MIN;
        for i in 4800..9600 {
            let t = i as f64 / sample_rate;
            let sample = amplitude * (2.0 * PI * freq * t).sin();
            let env = hilbert.envelope(sample);
            min_env = min_env.min(env);
            max_env = max_env.max(env);
        }

        let ripple = max_env - min_env;
        assert!(
            ripple < 0.05,
            "Hilbert envelope ripple {ripple:.4} too large for 440Hz sine"
        );
        let mean = (min_env + max_env) / 2.0;
        assert!(
            (mean - amplitude).abs() < 0.05,
            "Hilbert envelope mean {mean:.4} should be close to amplitude {amplitude}"
        );
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
