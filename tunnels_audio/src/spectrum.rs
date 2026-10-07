//! The spectrum: every band of the resonator bank as a level from 0 to 1,
//! whitened so that typical music reads roughly even across the bands.
//!
//! Each band's envelope is followed sample by sample, with an instant attack
//! and a 50 ms release, so a band's level does not depend on where the music
//! falls against the buffers. At the end of each buffer:
//!
//! 1. each band's level is taken in dB per octave, so bands of different
//!    widths compare;
//! 2. the corpus EQ, [`CORPUS_LEVEL_DB`], is taken off, leaving each band's
//!    deviation from typical music;
//! 3. the tilt: the slope, against octave position, of a 30 s average of
//!    that deviation, limited to ±3 dB per octave, is taken off too, so a
//!    bright or dark track still reads even. The average holds still while
//!    every band is below −60 dB per octave;
//! 4. one motion-clocked ceiling follows the loudest band, and every band
//!    reads as its place in a 30 dB window under the ceiling.

use std::time::Duration;

use tunnels_lib::audio::UnipolarF32;

use crate::bank::{self, NUM_BANDS, ResonatorBank};
use crate::processor::{MotionCeiling, UpdateRate, halflife_to_coeff};

/// Each band's median level over a corpus of music, in dB per octave.
///
/// Measured by `examples/spectrum_corpus.rs` on 156 recordings (two-minute
/// excerpts of 12 tracks of varied genres, and 144 MUSDB18 excerpts of a few
/// seconds each) through this crate's trim, bank and band followers: per
/// recording, each band's median level over the buffers where any band was
/// above [`SILENCE_DB`], then each band's median over the recordings.
pub const CORPUS_LEVEL_DB: [f32; NUM_BANDS] = [
    -17.62, -14.86, -18.23, -18.07, -18.30, -18.10, -18.01, -18.07, -18.67, -19.78, -19.91, -20.43,
    -20.31, -20.81, -20.54, -20.76, -21.01, -21.64, -23.79, -26.35, -26.45, -27.12, -27.91, -29.42,
    -31.20, -34.87,
];

/// Level, in dB per octave, below which every band must be for the input to
/// count as silent.
pub const SILENCE_DB: f32 = -60.0;

/// The spectrum stage: follows the bank sample by sample and reports every
/// band once per buffer.
pub(crate) struct Spectrum {
    /// Whether each band can be heard at the sample rate. A band that cannot
    /// reads zero and takes no part in the tilt or the ceiling.
    live: [bool; NUM_BANDS],
    /// Each band's envelope at the latest sample.
    envelopes: [f32; NUM_BANDS],
    /// The envelopes' per-sample release coefficient.
    release: f32,
    /// Each band's width, as dB per octave.
    width_db: [f32; NUM_BANDS],
    /// Each band's centre, in octaves from 1 kHz.
    octave: [f32; NUM_BANDS],
    /// Each live band's octave position less the live bands' mean, and zero
    /// for the rest: the regressor of the tilt's least-squares slope.
    octave_centred: [f32; NUM_BANDS],
    /// The sum of the squares of `octave_centred`.
    octave_spread: f32,
    /// Each band's level in dB per octave at the end of the latest buffer.
    levels_db: [f32; NUM_BANDS],
    /// Each band's slow average deviation from the corpus, in dB.
    deviation: [f32; NUM_BANDS],
    /// The deviation average's per-buffer coefficient, and the buffer rate it
    /// was derived for.
    tilt_coeff: f32,
    buffer_rate: f32,
    /// Follows the loudest whitened band, as a linear level.
    ceiling: MotionCeiling,
}

impl Spectrum {
    /// Half-life of each band's envelope release.
    const RELEASE_HALFLIFE: Duration = Duration::from_millis(50);
    /// Time constant of the average the tilt is measured from, in seconds.
    const TILT_TIME_CONSTANT: f32 = 30.0;
    /// The largest tilt taken off, in dB per octave either way.
    const TILT_LIMIT: f32 = 3.0;
    /// The range of levels under the ceiling that reads from 0 to 1, in dB.
    const WINDOW_DB: f32 = 30.0;
    /// The ceiling before anything has been heard, as a linear level relative
    /// to the corpus.
    const INITIAL_CEILING: f32 = 0.5;
    /// The lowest level, relative to the corpus, whose motion wears the
    /// ceiling down.
    const CEILING_GATE: f32 = 0.01;
    /// The smallest envelope a level is taken from, so silence reads as a
    /// finite level.
    const MIN_ENVELOPE: f32 = 1e-9;

    pub(crate) fn new(live: [bool; NUM_BANDS], sample_rate: f32) -> Self {
        let octave: [f32; NUM_BANDS] = std::array::from_fn(|b| (bank::centre(b) / 1000.0).log2());
        let live_count = live.iter().filter(|&&l| l).count().max(1) as f32;
        let mean_octave = (0..NUM_BANDS)
            .filter(|&b| live[b])
            .map(|b| octave[b])
            .sum::<f32>()
            / live_count;
        let octave_centred = std::array::from_fn(|b| {
            if live[b] {
                octave[b] - mean_octave
            } else {
                0.0
            }
        });
        Self {
            live,
            envelopes: [0.0; NUM_BANDS],
            release: halflife_to_coeff(
                Self::RELEASE_HALFLIFE,
                UpdateRate::new(sample_rate as u32, 1),
            ),
            width_db: std::array::from_fn(|b| 10.0 * bank::width_octaves(b).log10()),
            octave,
            octave_centred,
            octave_spread: octave_centred.iter().map(|x| x * x).sum(),
            levels_db: [20.0 * Self::MIN_ENVELOPE.log10(); NUM_BANDS],
            deviation: [0.0; NUM_BANDS],
            tilt_coeff: 0.0,
            buffer_rate: 0.0,
            ceiling: MotionCeiling::new(Self::INITIAL_CEILING, Self::CEILING_GATE),
        }
    }

    /// Follow every band's envelope at one sample of the bank.
    #[inline]
    pub(crate) fn push_sample(&mut self, bank: &ResonatorBank) {
        self.push(|band| bank.magnitude(band));
    }

    /// Follow every band's envelope at one sample, given each band's
    /// magnitude.
    #[inline]
    fn push(&mut self, magnitude: impl Fn(usize) -> f32) {
        let release = self.release;
        for (band, envelope) in self.envelopes.iter_mut().enumerate() {
            let magnitude = magnitude(band);
            *envelope = magnitude.max(release * *envelope + (1.0 - release) * magnitude);
        }
    }

    /// Each band's level in dB per octave at the end of the latest buffer.
    pub(crate) fn levels_db(&self) -> [f32; NUM_BANDS] {
        self.levels_db
    }

    /// Every band's output for the buffer just ended, given the buffer rate
    /// and the ceiling's forgetting rate in nepers per neper of motion.
    pub(crate) fn finish(&mut self, buffer_rate: f32, forget: f32) -> [UnipolarF32; NUM_BANDS] {
        if buffer_rate != self.buffer_rate {
            self.buffer_rate = buffer_rate;
            self.tilt_coeff = if buffer_rate > 0.0 {
                (-1.0 / (Self::TILT_TIME_CONSTANT * buffer_rate)).exp()
            } else {
                1.0
            };
        }

        let mut loudest = f32::NEG_INFINITY;
        for (band, level) in self.levels_db.iter_mut().enumerate() {
            *level =
                20.0 * self.envelopes[band].max(Self::MIN_ENVELOPE).log10() - self.width_db[band];
            if self.live[band] {
                loudest = loudest.max(*level);
            }
        }

        // Each band's deviation from the corpus, and the tilt: the
        // least-squares slope of the deviation's slow average against octave
        // position, held while the input is silent.
        let current: [f32; NUM_BANDS] =
            std::array::from_fn(|band| self.levels_db[band] - CORPUS_LEVEL_DB[band]);
        let listening = loudest > SILENCE_DB;
        let mut slope = 0.0;
        for (band, deviation) in self.deviation.iter_mut().enumerate() {
            if !self.live[band] {
                continue;
            }
            if listening {
                *deviation += (1.0 - self.tilt_coeff) * (current[band] - *deviation);
            }
            slope += self.octave_centred[band] * *deviation;
        }
        let slope = if self.octave_spread > 0.0 {
            (slope / self.octave_spread).clamp(-Self::TILT_LIMIT, Self::TILT_LIMIT)
        } else {
            0.0
        };

        let whitened: [f32; NUM_BANDS] =
            std::array::from_fn(|band| current[band] - slope * self.octave[band]);
        let top_db = (0..NUM_BANDS)
            .filter(|&band| self.live[band])
            .map(|band| whitened[band])
            .fold(f32::NEG_INFINITY, f32::max);

        let top = 10f32.powf(top_db / 20.0);
        let ceiling_db = 20.0 * self.ceiling.step(top, forget).log10();
        std::array::from_fn(|band| {
            if self.live[band] {
                UnipolarF32::new(1.0 + (whitened[band] - ceiling_db) / Self::WINDOW_DB)
            } else {
                UnipolarF32::ZERO
            }
        })
    }

    /// The slope taken off at the latest buffer, in dB per octave.
    #[cfg(test)]
    fn tilt(&self) -> f32 {
        let slope = (0..NUM_BANDS)
            .map(|b| self.octave_centred[b] * self.deviation[b])
            .sum::<f32>();
        (slope / self.octave_spread).clamp(-Self::TILT_LIMIT, Self::TILT_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;
    const BUFFER_RATE: f32 = 750.0;
    /// The ceiling's forgetting rate at the default eight-second Peak Memory.
    const FORGET: f32 = std::f32::consts::LN_2 / (4.2 * 8.0);

    /// Hold every band at a level, in dB per octave, for `secs`, pushing
    /// `samples` samples per buffer, and return the outputs of the last
    /// buffer. A steady level reads the same at any number of samples.
    fn hold(
        spectrum: &mut Spectrum,
        level_db: impl Fn(usize) -> f32,
        secs: f32,
        samples: usize,
    ) -> [UnipolarF32; NUM_BANDS] {
        let magnitudes: [f32; NUM_BANDS] = std::array::from_fn(|b| {
            10f32.powf((level_db(b) + 10.0 * bank::width_octaves(b).log10()) / 20.0)
        });
        let mut out = [UnipolarF32::ZERO; NUM_BANDS];
        for _ in 0..(secs * BUFFER_RATE) as usize {
            for _ in 0..samples {
                spectrum.push(|b| magnitudes[b]);
            }
            out = spectrum.finish(BUFFER_RATE, FORGET);
        }
        out
    }

    /// The corpus, tilted by `slope` dB per octave about 1 kHz.
    fn tilted(slope: f32) -> impl Fn(usize) -> f32 {
        move |b| CORPUS_LEVEL_DB[b] + slope * (bank::centre(b) / 1000.0).log2()
    }

    /// The least-squares slope of the outputs against octave position, in dB
    /// per octave.
    fn output_slope(out: &[UnipolarF32; NUM_BANDS]) -> f32 {
        let octave: [f32; NUM_BANDS] = std::array::from_fn(|b| (bank::centre(b) / 1000.0).log2());
        let mean = octave.iter().sum::<f32>() / NUM_BANDS as f32;
        let (num, den) = (0..NUM_BANDS).fold((0.0, 0.0), |(num, den), b| {
            let x = octave[b] - mean;
            (num + x * out[b].val(), den + x * x)
        });
        Spectrum::WINDOW_DB * num / den
    }

    /// Music shaped like the corpus reads even, and stays even when it is
    /// brighter or darker than the corpus: the tilt follows it, up to the
    /// limit. A tilt beyond the limit is pulled back by the limit and no
    /// further.
    #[test]
    fn a_tilt_is_pulled_back_toward_flat_within_the_limit() {
        let mut spectrum = Spectrum::new([true; NUM_BANDS], SAMPLE_RATE);
        let out = hold(&mut spectrum, tilted(0.0), 1.0, 1);
        for (band, v) in out.iter().enumerate() {
            assert!(
                v.val() > 0.999,
                "band {band} of the corpus reads {}",
                v.val()
            );
        }

        for slope in [2.0, -2.0, 5.0, -5.0] {
            let mut spectrum = Spectrum::new([true; NUM_BANDS], SAMPLE_RATE);
            let first = hold(&mut spectrum, tilted(slope), 1.0 / BUFFER_RATE, 1);
            // A steeper tilt spans more than the window, and the bands below
            // it read zero.
            if slope.abs() < Spectrum::TILT_LIMIT {
                assert!(
                    (output_slope(&first) - slope).abs() < 0.05,
                    "{slope} dB/oct reads {} dB/oct before the tilt has moved",
                    output_slope(&first)
                );
            }
            let settled = hold(&mut spectrum, tilted(slope), 180.0, 1);
            let taken_off = slope.clamp(-Spectrum::TILT_LIMIT, Spectrum::TILT_LIMIT);
            assert!(
                (spectrum.tilt() - taken_off).abs() < 0.02,
                "{slope} dB/oct: tilt {} taken off, not {taken_off}",
                spectrum.tilt()
            );
            let residual = output_slope(&settled);
            assert!(
                (residual - (slope - taken_off)).abs() < 0.05,
                "{slope} dB/oct reads {residual} dB/oct once settled"
            );
        }
    }

    /// Silence leaves the tilt where it was and reads zero in every band; a
    /// band the sample rate cannot carry reads zero and does not count
    /// toward the tilt.
    #[test]
    fn silence_holds_the_tilt_and_reads_zero() {
        let mut live = [true; NUM_BANDS];
        live[NUM_BANDS - 1] = false;
        let mut spectrum = Spectrum::new(live, SAMPLE_RATE);
        let out = hold(&mut spectrum, tilted(2.0), 60.0, 1);
        assert_eq!(out[NUM_BANDS - 1], UnipolarF32::ZERO, "a dead band");
        let tilt = spectrum.tilt();
        assert!(tilt > 1.5, "the tilt has followed the music: {tilt}");

        // A second for the bands' release to carry them under the silence
        // threshold, at the real number of samples per buffer.
        let silence = |_| f32::NEG_INFINITY;
        hold(&mut spectrum, silence, 1.0, 64);
        let tilt = spectrum.tilt();
        let out = hold(&mut spectrum, silence, 60.0, 64);
        assert_eq!(spectrum.tilt(), tilt, "silence moved the tilt");
        assert_eq!(out, [UnipolarF32::ZERO; NUM_BANDS], "silence reads zero");
    }
}
