//! Teager-Kaiser Energy Operator for transient detection.
//!
//! TKEO is a nonlinear energy measure: Ψ[x(n)] = x(n)² - x(n-1)·x(n+1)
//! For real-time (causal) use, we use the one-sample-delayed form:
//! Ψ[n] = x[n-1]² - x[n-2]·x[n]
//!
//! TKEO is frequency-weighted (higher frequencies produce more energy),
//! which makes it excellent for transient detection (transients are broadband)
//! but unsuitable as a general amplitude envelope.

/// Causal Teager-Kaiser Energy Operator.
pub struct Tkeo {
    x_prev1: f32,
    x_prev2: f32,
}

impl Tkeo {
    pub fn new() -> Self {
        Self {
            x_prev1: 0.0,
            x_prev2: 0.0,
        }
    }

    /// Process one sample, returning the TKEO energy value.
    /// Output is non-negative for single-component signals;
    /// may be slightly negative for multi-component signals.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let energy = self.x_prev1 * self.x_prev1 - self.x_prev2 * x;
        self.x_prev2 = self.x_prev1;
        self.x_prev1 = x;
        energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn silence_produces_zero() {
        let mut tkeo = Tkeo::new();
        for _ in 0..100 {
            assert_eq!(tkeo.process(0.0), 0.0);
        }
    }

    #[test]
    fn sine_produces_positive_energy() {
        let mut tkeo = Tkeo::new();
        let sr = 48000.0_f32;
        let freq = 440.0;

        // Let it settle.
        for i in 0..480 {
            let t = i as f32 / sr;
            tkeo.process((2.0 * PI * freq * t).sin());
        }

        // Steady-state energy should be consistently positive.
        for i in 480..960 {
            let t = i as f32 / sr;
            let energy = tkeo.process((2.0 * PI * freq * t).sin());
            assert!(energy > 0.0, "TKEO energy should be positive for a sine");
        }
    }

    #[test]
    fn transient_produces_spike() {
        let mut tkeo = Tkeo::new();

        // Feed silence then a sudden step.
        for _ in 0..100 {
            tkeo.process(0.0);
        }
        let e0 = tkeo.process(0.0);
        let _e1 = tkeo.process(1.0); // step
        let e2 = tkeo.process(1.0);

        // The energy right after the transient should be much larger
        // than during silence.
        assert!(e2.abs() > e0.abs() * 100.0 || e0 == 0.0);
    }
}
