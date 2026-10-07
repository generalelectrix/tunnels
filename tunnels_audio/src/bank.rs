//! A constant-Q bank of complex resonators.
//!
//! Each band is `ORDER` identical complex one-pole resonators in cascade,
//! `y[n] = (1 - |p|) x[n] + p y[n-1]`, centred on the band's frequency. The
//! output of a complex resonator is analytic, so its magnitude is the band's
//! envelope with no Hilbert stage. It passes only the positive-frequency half
//! of a real signal, so envelopes are doubled: a tone of amplitude A at a
//! band's centre reads A. Every band steps together, one input
//! sample at a time, so the inner loops run across bands and vectorize.
//!
//! The two lowest bands are an octave wide, matching the resolution the low
//! end can have at a short latency; above 250 Hz the bands are a quarter
//! octave, up to 16 kHz.
//!
//! A complex one-pole has no zero, so far from its centre it passes about
//! `1 - |p|` of a signal however distant the signal is: for a band wide in
//! hertz that floor is high, and a loud low end would leak into the treble at
//! around -50 dB. The first stage of every quarter-octave band therefore
//! carries a zero at DC, `y[n] = g (x[n] - x[n-1]) + p y[n-1]`, which pushes a
//! low tone's leakage into the high bands below -80 dB while tilting the
//! band's own shape by under a dB.

/// Number of bands, in ascending frequency order.
pub const NUM_BANDS: usize = 26;

/// Resonators in cascade per band. More stages steepen a band's skirts at the
/// same bandwidth, at the cost of a little more delay.
const ORDER: usize = 4;

/// Bands padded to a multiple of the vector width, so the padding lanes can be
/// stepped with the rest instead of peeled off.
const LANES: usize = 28;

/// Lower edge of the quarter-octave bands, in Hz.
const QUARTER_OCTAVE_FROM: f32 = 250.0;

/// Centre of each band in Hz: two octave-wide bands below
/// `QUARTER_OCTAVE_FROM`, then quarter octaves starting an eighth of an octave
/// above it.
pub fn centre(band: usize) -> f32 {
    match band {
        0 => 70.0,
        1 => 150.0,
        _ => QUARTER_OCTAVE_FROM * 2f32.powf((band - 2) as f32 / 4.0 + 1.0 / 8.0),
    }
}

/// Width of each band in octaves.
pub fn width_octaves(band: usize) -> f32 {
    if band < 2 { 1.0 } else { 0.25 }
}

/// The -3 dB bandwidth of a band in Hz.
pub fn bandwidth(band: usize) -> f32 {
    let half = width_octaves(band) / 2.0;
    centre(band) * (2f32.powf(half) - 2f32.powf(-half))
}

pub struct ResonatorBank {
    pole_re: [f32; LANES],
    pole_im: [f32; LANES],
    /// Input gain per stage, `1 - |p|`, which gives the cascade unit gain at
    /// its centre. Zero for bands above the sample rate's reach, and for the
    /// padding lanes.
    gain: [f32; LANES],
    /// The first stage's input gain, which for a band with a zero at DC also
    /// undoes the zero's gain at the band's centre.
    first_gain: [f32; LANES],
    /// One where the first stage subtracts the previous input (a zero at DC),
    /// zero where it does not.
    dc_zero: [f32; LANES],
    previous_sample: f32,
    state_re: [[f32; LANES]; ORDER],
    state_im: [[f32; LANES]; ORDER],
    /// Largest squared magnitude per band since the peaks were last taken.
    peak_sq: [f32; LANES],
}

impl ResonatorBank {
    pub fn new(sample_rate: f32) -> Self {
        let mut bank = Self {
            pole_re: [0.0; LANES],
            pole_im: [0.0; LANES],
            gain: [0.0; LANES],
            first_gain: [0.0; LANES],
            dc_zero: [0.0; LANES],
            previous_sample: 0.0,
            state_re: [[0.0; LANES]; ORDER],
            state_im: [[0.0; LANES]; ORDER],
            peak_sq: [0.0; LANES],
        };
        // The cascade's bandwidth narrows with each stage, so each stage is
        // made wider by the factor that brings the cascade back to the band's.
        let widen = 1.0 / (2f32.powf(1.0 / ORDER as f32) - 1.0).sqrt();
        for band in 0..NUM_BANDS {
            let (fc, bw) = (centre(band), bandwidth(band));
            if fc + bw / 2.0 >= 0.45 * sample_rate {
                continue;
            }
            let r = (-std::f32::consts::PI * bw * widen / sample_rate).exp();
            let w = std::f32::consts::TAU * fc / sample_rate;
            bank.pole_re[band] = r * w.cos();
            bank.pole_im[band] = r * w.sin();
            bank.gain[band] = 1.0 - r;
            if width_octaves(band) < 1.0 {
                bank.dc_zero[band] = 1.0;
                bank.first_gain[band] = (1.0 - r) / (2.0 * (w / 2.0).sin());
            } else {
                bank.first_gain[band] = 1.0 - r;
            }
        }
        bank
    }

    /// Whether a band can be heard at this sample rate. A band whose upper
    /// edge comes near Nyquist is silent rather than aliased.
    pub fn is_live(&self, band: usize) -> bool {
        self.gain[band] > 0.0
    }

    /// Run one input sample through every band.
    #[inline]
    pub fn push(&mut self, sample: f32) {
        let mut in_re: [f32; LANES] = std::array::from_fn(|l| {
            (sample - self.dc_zero[l] * self.previous_sample) * self.first_gain[l]
        });
        let mut in_im = [0.0_f32; LANES];
        self.previous_sample = sample;
        for stage in 0..ORDER {
            let gain = if stage == 0 { [1.0; LANES] } else { self.gain };
            let (sr, si) = (&mut self.state_re[stage], &mut self.state_im[stage]);
            for l in 0..LANES {
                let (xr, xi) = (in_re[l] * gain[l], in_im[l] * gain[l]);
                let (yr, yi) = (sr[l], si[l]);
                let re = xr + self.pole_re[l] * yr - self.pole_im[l] * yi;
                let im = xi + self.pole_re[l] * yi + self.pole_im[l] * yr;
                sr[l] = re;
                si[l] = im;
                in_re[l] = re;
                in_im[l] = im;
            }
        }
        for l in 0..LANES {
            self.peak_sq[l] = self.peak_sq[l].max(in_re[l] * in_re[l] + in_im[l] * in_im[l]);
        }
    }

    /// A band's envelope at the latest sample.
    #[inline]
    pub fn magnitude(&self, band: usize) -> f32 {
        let (re, im) = (
            self.state_re[ORDER - 1][band],
            self.state_im[ORDER - 1][band],
        );
        2.0 * (re * re + im * im).sqrt()
    }

    /// Each band's peak envelope since the last call, and start the next.
    pub fn take_peaks(&mut self) -> [f32; NUM_BANDS] {
        let peaks = std::array::from_fn(|band| 2.0 * self.peak_sq[band].sqrt());
        self.peak_sq = [0.0; LANES];
        peaks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady tone at a band's centre reads as its amplitude there, and
    /// much less in bands well clear of it: an octave for the quarter-octave
    /// bands, two for the octave-wide ones, whose skirts reach further. A loud
    /// low tone does not leak into the treble. Bands past what
    /// the sample rate can carry stay silent.
    #[test]
    fn a_tone_lands_in_its_own_band() {
        let sample_rate = 48_000.0;
        for band in [0, 1, 6, 14, 22] {
            let mut bank = ResonatorBank::new(sample_rate);
            let f = centre(band);
            for i in 0..(sample_rate as usize / 2) {
                bank.push((std::f32::consts::TAU * f * i as f32 / sample_rate).sin());
            }
            bank.take_peaks();
            for i in 0..4800 {
                let n = sample_rate as usize / 2 + i;
                bank.push((std::f32::consts::TAU * f * n as f32 / sample_rate).sin());
            }
            let peaks = bank.take_peaks();
            assert!(
                (peaks[band] - 1.0).abs() < 0.05,
                "band {band}: {}",
                peaks[band]
            );
            for (other, &p) in peaks.iter().enumerate() {
                let clear = 2.0 * width_octaves(band).max(width_octaves(other)).max(0.5);
                if (centre(other) / f).log2().abs() >= clear {
                    assert!(p < 0.2, "band {other} hears band {band}'s tone at {p}");
                }
            }
        }

        // Measured while the tone is still sounding: stopping it would be a
        // step, which really does reach the treble.
        let mut bank = ResonatorBank::new(sample_rate);
        let mut leak = [0.0; NUM_BANDS];
        for i in 0..(sample_rate as usize) {
            bank.push(0.8 * (std::f32::consts::TAU * 55.0 * i as f32 / sample_rate).sin());
            if i % 64 == 63 {
                leak = bank.take_peaks();
            }
        }
        for (band, level) in leak.iter().enumerate().skip(16) {
            let db = 20.0 * level.log10();
            assert!(
                db < -75.0,
                "a 55 Hz tone leaks into band {band} at {db:.0} dBFS"
            );
        }

        let bank = ResonatorBank::new(16_000.0);
        assert!(bank.is_live(14), "a 2.6 kHz band fits at 16 kHz");
        assert!(!bank.is_live(NUM_BANDS - 1), "a 15 kHz band does not");
    }
}
