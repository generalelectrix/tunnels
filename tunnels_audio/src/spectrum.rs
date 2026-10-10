//! The spectrum: every band of the resonator bank as a level from 0 to 1,
//! whitened so that typical music reads roughly even across the bands.
//!
//! Each band's envelope is followed sample by sample, with an instant attack,
//! a 50 ms hold at each peak and then a 50 ms release, so a band's level does
//! not depend on where the music falls against the buffers. At the end of each
//! buffer:
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
//!    reads as its place in a 30 dB window under the ceiling;
//! 5. each band's reading passes through a one-pole low-pass with a 30 Hz
//!    corner, [`OUTPUT_CORNER`], stepped once per buffer.

use std::time::Duration;

use tunnels_lib::audio::UnipolarF32;

use crate::bank::{self, NUM_BANDS, ResonatorBank};
use crate::processor::{MotionCeiling, UpdateRate, halflife_to_coeff, secs_to_duration};

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

/// The corner of the low-pass every band's output passes through.
pub const OUTPUT_CORNER: CornerFrequency = CornerFrequency::from_hz(30.0);

/// The corner frequency of a one-pole low-pass, in Hz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerFrequency(f32);

impl CornerFrequency {
    pub const fn from_hz(hz: f32) -> Self {
        Self(hz)
    }

    pub const fn as_hz(self) -> f32 {
        self.0
    }

    /// The time constant of a one-pole low-pass with this corner,
    /// 1 / (2π · corner).
    pub fn time_constant(self) -> Duration {
        secs_to_duration(1.0 / (std::f32::consts::TAU * self.0))
    }

    /// The coefficient of a one-pole low-pass with this corner, stepped at
    /// `update_rate`: the fraction of the output kept each step, in [0, 1),
    /// so that a step in the input is 63% complete after one time constant.
    /// Zero, passing the input through, at a rate that is not positive.
    fn one_pole_coeff(self, update_rate: UpdateRate) -> f32 {
        let update_rate = update_rate.as_hz();
        if update_rate <= 0.0 {
            return 0.0;
        }
        (-std::f32::consts::TAU * self.0 / update_rate).exp()
    }
}

/// The spectrum stage: follows the bank sample by sample and reports every
/// band once per buffer.
pub(crate) struct Spectrum {
    /// Whether each band can be heard at the sample rate. A band that cannot
    /// reads zero and takes no part in the tilt or the ceiling.
    live: [bool; NUM_BANDS],
    /// Each band's width, as dB per octave.
    width_db: [f32; NUM_BANDS],
    /// Each band's centre, in octaves from 1 kHz.
    octave: [f32; NUM_BANDS],
    /// Every band's envelope.
    followers: BandFollowers,
    /// The tilt taken off every band's deviation from the corpus.
    tilt: Tilt,
    /// Follows the loudest whitened band, as a linear level.
    ceiling: MotionCeiling,
    /// The low-pass every band's output passes through.
    output_filter: OutputFilter,
    /// Each band's level in dB per octave at the end of the latest buffer.
    levels_db: [f32; NUM_BANDS],
}

impl Spectrum {
    /// The range of levels under the ceiling that reads from 0 to 1, in dB.
    const WINDOW_DB: f32 = 30.0;
    /// The ceiling before anything has been heard, as a linear level relative
    /// to the corpus.
    const INITIAL_CEILING: UnipolarF32 = UnipolarF32::new(0.5);
    /// The lowest level, relative to the corpus, whose motion wears the
    /// ceiling down.
    const CEILING_GATE: UnipolarF32 = UnipolarF32::new(0.01);

    pub(crate) fn new(live: [bool; NUM_BANDS], sample_rate: UpdateRate) -> Self {
        let octave: [f32; NUM_BANDS] = std::array::from_fn(|b| (bank::centre(b) / 1000.0).log2());
        Self {
            live,
            width_db: std::array::from_fn(|b| 10.0 * bank::width_octaves(b).log10()),
            octave,
            followers: BandFollowers::new(sample_rate),
            tilt: Tilt::new(live, &octave),
            ceiling: MotionCeiling::new(Self::INITIAL_CEILING, Self::CEILING_GATE),
            output_filter: OutputFilter::new(OUTPUT_CORNER),
            levels_db: [20.0 * BandFollowers::MIN_ENVELOPE.log10(); NUM_BANDS],
        }
    }

    /// Follow every band's envelope at one sample of the bank.
    #[inline]
    pub(crate) fn push_sample(&mut self, bank: &ResonatorBank) {
        self.followers.push(|band| bank.magnitude(band));
    }

    /// Each band's level in dB per octave at the end of the latest buffer.
    pub(crate) fn levels_db(&self) -> [f32; NUM_BANDS] {
        self.levels_db
    }

    /// Every band's output for the buffer just ended, given the buffer rate
    /// and the ceiling's forgetting rate in nepers per neper of motion.
    pub(crate) fn finish(
        &mut self,
        buffer_rate: UpdateRate,
        forget: f32,
    ) -> [UnipolarF32; NUM_BANDS] {
        self.levels_db = self.followers.levels_db(&self.width_db);
        let deviation: [f32; NUM_BANDS] =
            std::array::from_fn(|band| self.levels_db[band] - CORPUS_LEVEL_DB[band]);
        let slope = self.tilt.update(&deviation, self.listening(), buffer_rate);
        let whitened: [f32; NUM_BANDS] =
            std::array::from_fn(|band| deviation[band] - slope * self.octave[band]);

        let top_db = self
            .live_bands()
            .map(|band| whitened[band])
            .fold(f32::NEG_INFINITY, f32::max);
        let top = 10f32.powf(top_db / 20.0);
        let ceiling_db = 20.0 * self.ceiling.step(top, forget).log10();
        let windowed = std::array::from_fn(|band| {
            if self.live[band] {
                UnipolarF32::new(1.0 + (whitened[band] - ceiling_db) / Self::WINDOW_DB)
            } else {
                UnipolarF32::ZERO
            }
        });
        self.output_filter.step(&windowed, buffer_rate)
    }

    /// Whether any live band was above [`SILENCE_DB`] at the end of the latest
    /// buffer.
    fn listening(&self) -> bool {
        self.live_bands()
            .any(|band| self.levels_db[band] > SILENCE_DB)
    }

    /// The indices of the bands that can be heard at the sample rate.
    fn live_bands(&self) -> impl Iterator<Item = usize> + '_ {
        (0..NUM_BANDS).filter(|&band| self.live[band])
    }
}

/// Every band's envelope, followed sample by sample: a magnitude at or above
/// the envelope sets it at once and starts a [`BandFollowers::HOLD`] in which
/// the envelope stays put, and once the hold has run out the envelope falls
/// with a [`BandFollowers::RELEASE_HALFLIFE`] release.
pub(crate) struct BandFollowers {
    /// Each band's envelope at the latest sample.
    envelopes: [f32; NUM_BANDS],
    /// The samples left in each band's hold.
    hold_left: [u32; NUM_BANDS],
    /// The hold, in samples.
    hold: u32,
    /// The release coefficient, in [0, 1): the fraction of an envelope kept
    /// each sample while its band is falling.
    release: f32,
}

impl BandFollowers {
    /// How long each envelope holds a peak before it releases.
    const HOLD: Duration = Duration::from_millis(50);
    /// Half-life of each envelope's release.
    const RELEASE_HALFLIFE: Duration = Duration::from_millis(50);
    /// The smallest envelope a level is taken from, so silence reads as a
    /// finite level.
    const MIN_ENVELOPE: f32 = 1e-9;

    /// Every envelope at zero, held and released at the sample rate.
    pub(crate) fn new(sample_rate: UpdateRate) -> Self {
        Self {
            envelopes: [0.0; NUM_BANDS],
            hold_left: [0; NUM_BANDS],
            hold: (Self::HOLD.as_secs_f32() * sample_rate.as_hz()).round() as u32,
            release: halflife_to_coeff(Self::RELEASE_HALFLIFE, sample_rate),
        }
    }

    /// Follow every band's envelope at one sample, given each band's
    /// magnitude.
    #[inline]
    pub(crate) fn push(&mut self, magnitude: impl Fn(usize) -> f32) {
        let release = self.release;
        for (band, (envelope, hold_left)) in self
            .envelopes
            .iter_mut()
            .zip(self.hold_left.iter_mut())
            .enumerate()
        {
            let magnitude = magnitude(band);
            if magnitude >= *envelope {
                *envelope = magnitude;
                *hold_left = self.hold;
            } else if *hold_left > 0 {
                *hold_left -= 1;
            } else {
                *envelope = release * *envelope + (1.0 - release) * magnitude;
            }
        }
    }

    /// Each band's envelope as a level in dB per octave, given each band's
    /// width in dB per octave.
    pub(crate) fn levels_db(&self, width_db: &[f32; NUM_BANDS]) -> [f32; NUM_BANDS] {
        std::array::from_fn(|band| {
            20.0 * self.envelopes[band].max(Self::MIN_ENVELOPE).log10() - width_db[band]
        })
    }
}

/// A one-pole low-pass on every band's output, stepped once per buffer at
/// that buffer's rate. An output below [`OutputFilter::FLOOR`] reads zero, so
/// a band settles to exactly zero.
struct OutputFilter {
    /// The filter's corner.
    corner: CornerFrequency,
    /// The coefficient, in [0, 1): the fraction of each output kept each
    /// buffer.
    coeff: f32,
    /// The buffer rate `coeff` was derived for, if it has been.
    buffer_rate: Option<UpdateRate>,
    /// Each band's output at the latest buffer.
    outputs: [f32; NUM_BANDS],
}

impl OutputFilter {
    /// The smallest output that does not read zero.
    const FLOOR: f32 = 1e-6;

    /// Every output at zero.
    fn new(corner: CornerFrequency) -> Self {
        Self {
            corner,
            coeff: 0.0,
            buffer_rate: None,
            outputs: [0.0; NUM_BANDS],
        }
    }

    /// Step every band's output toward its input over one buffer at
    /// `buffer_rate`, and return the outputs.
    fn step(
        &mut self,
        input: &[UnipolarF32; NUM_BANDS],
        buffer_rate: UpdateRate,
    ) -> [UnipolarF32; NUM_BANDS] {
        if self.buffer_rate != Some(buffer_rate) {
            self.buffer_rate = Some(buffer_rate);
            self.coeff = self.corner.one_pole_coeff(buffer_rate);
        }
        let gain = 1.0 - self.coeff;
        for (output, input) in self.outputs.iter_mut().zip(input) {
            *output += gain * (input.val() - *output);
            if *output < Self::FLOOR {
                *output = 0.0;
            }
        }
        self.outputs.map(UnipolarF32::new)
    }
}

/// The tilt of the spectrum against the corpus: the least-squares slope,
/// against octave position, of every live band's slow average deviation from
/// the corpus, limited to [`Tilt::LIMIT`] either way. The average holds still
/// through buffers in which nothing is heard.
pub(crate) struct Tilt {
    /// Whether each band takes part in the tilt.
    live: [bool; NUM_BANDS],
    /// Each live band's octave position less the live bands' mean, and zero
    /// for the rest: the regressor of the slope.
    octave_centred: [f32; NUM_BANDS],
    /// The sum of the squares of `octave_centred`.
    octave_spread: f32,
    /// Each band's slow average deviation from the corpus, in dB.
    average: [f32; NUM_BANDS],
    /// The average's coefficient, in [0, 1]: the fraction of the average kept
    /// each buffer.
    coeff: f32,
    /// The buffer rate `coeff` was derived for, if it has been.
    buffer_rate: Option<UpdateRate>,
    /// The slope at the latest buffer, in dB per octave.
    slope: f32,
}

impl Tilt {
    /// Time constant of the average the slope is measured from.
    const TIME_CONSTANT: Duration = Duration::from_secs(30);
    /// The largest slope, in dB per octave either way.
    const LIMIT: f32 = 3.0;

    /// A flat tilt over the bands marked live, given each band's centre in
    /// octaves.
    pub(crate) fn new(live: [bool; NUM_BANDS], octave: &[f32; NUM_BANDS]) -> Self {
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
            octave_centred,
            octave_spread: octave_centred.iter().map(|x| x * x).sum(),
            average: [0.0; NUM_BANDS],
            coeff: 0.0,
            buffer_rate: None,
            slope: 0.0,
        }
    }

    /// Fold one buffer's deviation from the corpus, in dB per band, into the
    /// average if anything was heard in it (`listening`), and return the
    /// slope.
    pub(crate) fn update(
        &mut self,
        deviation: &[f32; NUM_BANDS],
        listening: bool,
        buffer_rate: UpdateRate,
    ) -> f32 {
        if self.buffer_rate != Some(buffer_rate) {
            self.buffer_rate = Some(buffer_rate);
            let buffer_rate = buffer_rate.as_hz();
            self.coeff = if buffer_rate > 0.0 {
                (-1.0 / (Self::TIME_CONSTANT.as_secs_f32() * buffer_rate)).exp()
            } else {
                1.0
            };
        }

        let mut slope = 0.0;
        for (band, average) in self.average.iter_mut().enumerate() {
            if !self.live[band] {
                continue;
            }
            if listening {
                *average += (1.0 - self.coeff) * (deviation[band] - *average);
            }
            slope += self.octave_centred[band] * *average;
        }
        self.slope = if self.octave_spread > 0.0 {
            (slope / self.octave_spread).clamp(-Self::LIMIT, Self::LIMIT)
        } else {
            0.0
        };
        self.slope
    }

    /// The slope at the latest buffer, in dB per octave.
    #[cfg(test)]
    pub(crate) fn slope(&self) -> f32 {
        self.slope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: UpdateRate = UpdateRate::new(48_000, 1);
    const BUFFER_RATE: UpdateRate = UpdateRate::new(48_000, 64);
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
        for _ in 0..(secs * BUFFER_RATE.as_hz()) as usize {
            for _ in 0..samples {
                spectrum.followers.push(|b| magnitudes[b]);
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
            // Long enough for the output filter to settle, and too short for
            // the tilt to move more than a fraction of a percent.
            let first = hold(&mut spectrum, tilted(slope), 0.1, 1);
            // A steeper tilt spans more than the window, and the bands below
            // it read zero.
            if slope.abs() < Tilt::LIMIT {
                assert!(
                    (output_slope(&first) - slope).abs() < 0.05,
                    "{slope} dB/oct reads {} dB/oct before the tilt has moved",
                    output_slope(&first)
                );
            }
            let settled = hold(&mut spectrum, tilted(slope), 180.0, 1);
            let taken_off = slope.clamp(-Tilt::LIMIT, Tilt::LIMIT);
            assert!(
                (spectrum.tilt.slope() - taken_off).abs() < 0.02,
                "{slope} dB/oct: tilt {} taken off, not {taken_off}",
                spectrum.tilt.slope()
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
        let tilt = spectrum.tilt.slope();
        assert!(tilt > 1.5, "the tilt has followed the music: {tilt}");

        // A second for the bands' release to carry them under the silence
        // threshold, at the real number of samples per buffer.
        let silence = |_| f32::NEG_INFINITY;
        hold(&mut spectrum, silence, 1.0, 64);
        let tilt = spectrum.tilt.slope();
        let out = hold(&mut spectrum, silence, 60.0, 64);
        assert_eq!(spectrum.tilt.slope(), tilt, "silence moved the tilt");
        assert_eq!(out, [UnipolarF32::ZERO; NUM_BANDS], "silence reads zero");
    }

    /// A peak holds its band's envelope for the hold, a new peak in the hold
    /// starts it again, and once it has run out the envelope falls at the
    /// release.
    #[test]
    fn a_peak_holds_then_releases() {
        let mut followers = BandFollowers::new(SAMPLE_RATE);
        let hold = (BandFollowers::HOLD.as_secs_f32() * SAMPLE_RATE.as_hz()).round() as usize;
        assert_eq!(hold, 2400, "50 ms at 48 kHz");
        let release = halflife_to_coeff(BandFollowers::RELEASE_HALFLIFE, SAMPLE_RATE);

        // One sample at 1, then silence; band 1 stays silent throughout.
        let push = |followers: &mut BandFollowers, m: f32| {
            followers.push(|b| if b == 0 { m } else { 0.0 });
        };
        push(&mut followers, 1.0);
        for i in 0..hold {
            push(&mut followers, 0.0);
            assert_eq!(followers.envelopes[0], 1.0, "sample {i} of the hold");
        }
        let mut expected = 1.0;
        for i in 0..hold {
            push(&mut followers, 0.0);
            expected *= release;
            assert!(
                (followers.envelopes[0] / expected - 1.0).abs() < 1e-4,
                "sample {i} of the release reads {}, not {expected}",
                followers.envelopes[0]
            );
        }
        assert!(
            (followers.envelopes[0] - 0.5).abs() < 1e-3,
            "a half-life after the hold the envelope is at {}",
            followers.envelopes[0]
        );
        assert_eq!(followers.envelopes[1], 0.0, "a silent band");

        // A peak, a lower magnitude that does not restart the hold, and a
        // peak equal to the envelope halfway through that does.
        let mut followers = BandFollowers::new(SAMPLE_RATE);
        push(&mut followers, 1.0);
        for i in 1..hold / 2 {
            push(&mut followers, if i == 10 { 0.9 } else { 0.0 });
        }
        push(&mut followers, 1.0);
        for i in 0..hold {
            push(&mut followers, 0.0);
            assert_eq!(
                followers.envelopes[0], 1.0,
                "sample {i} of the restarted hold"
            );
        }
        push(&mut followers, 0.0);
        assert_eq!(
            followers.envelopes[0], release,
            "the first sample of the release"
        );
    }

    /// A step into the output filter is 63% complete after one time constant
    /// of the corner, to within a buffer, at any buffer size, and settles on
    /// the step; an input that stays at zero reads exactly zero, and
    /// an output that decays toward zero reaches it.
    #[test]
    fn the_output_filter_follows_a_step_at_its_corner() {
        let tau = OUTPUT_CORNER.time_constant().as_secs_f32();
        assert!(
            (tau - 1.0 / (std::f32::consts::TAU * 30.0)).abs() < 1e-6,
            "time constant {tau} s"
        );
        let target = 1.0 - (-1.0f32).exp();
        let step: [UnipolarF32; NUM_BANDS] = std::array::from_fn(|b| {
            if b == 0 {
                UnipolarF32::ZERO
            } else {
                UnipolarF32::new(0.8)
            }
        });
        for frames in [16, 64, 256] {
            let buffer_rate = UpdateRate::new(48_000, frames);
            let mut filter = OutputFilter::new(OUTPUT_CORNER);
            let at_tau = tau * buffer_rate.as_hz();
            let mut previous = 0.0;
            for n in 1..=(at_tau.ceil() as usize) {
                let out = filter.step(&step, buffer_rate);
                assert_eq!(out[0], UnipolarF32::ZERO, "a band held at zero");
                let fraction = out[1].val() / 0.8;
                if n == at_tau.floor() as usize {
                    previous = fraction;
                }
                if n == at_tau.ceil() as usize {
                    assert!(
                        previous <= target && target <= fraction,
                        "{frames} frames per buffer: {previous} to {fraction} of the step \
                         either side of one time constant"
                    );
                }
            }
            let mut out = [UnipolarF32::ZERO; NUM_BANDS];
            for _ in 0..(buffer_rate.as_hz() as usize) {
                out = filter.step(&step, buffer_rate);
            }
            for (band, (out, step)) in out.iter().zip(&step).enumerate() {
                assert!(
                    (out.val() - step.val()).abs() < 1e-6,
                    "{frames} frames per buffer: band {band} settled at {}, not {}",
                    out.val(),
                    step.val()
                );
            }

            for _ in 0..(buffer_rate.as_hz() as usize) {
                out = filter.step(&[UnipolarF32::ZERO; NUM_BANDS], buffer_rate);
            }
            assert_eq!(
                out,
                [UnipolarF32::ZERO; NUM_BANDS],
                "{frames} frames per buffer: decayed"
            );
        }
    }

    /// The tilt fits only the live bands, moves by the average's share of
    /// each buffer, stops at the limit, and holds while the input is silent;
    /// with no live bands it stays flat.
    #[test]
    fn tilt_fits_the_live_bands_within_the_limit() {
        let octave: [f32; NUM_BANDS] = std::array::from_fn(|b| b as f32 / 3.0 - 6.0);
        let mut live = [true; NUM_BANDS];
        live[NUM_BANDS - 1] = false;
        // A line of `slope` dB per octave over the live bands, and a dead band
        // far off it.
        let line = |slope: f32| -> [f32; NUM_BANDS] {
            std::array::from_fn(|b| if live[b] { slope * octave[b] } else { 1000.0 })
        };
        let buffers = |secs: f32| (secs * BUFFER_RATE.as_hz()) as usize;
        let mut tilt = Tilt::new(live, &octave);

        let share = 1.0 - (-1.0 / (Tilt::TIME_CONSTANT.as_secs_f32() * BUFFER_RATE.as_hz())).exp();
        let first = tilt.update(&line(2.0), true, BUFFER_RATE);
        assert!(
            (first / (2.0 * share) - 1.0).abs() < 1e-3,
            "one buffer moved the tilt to {first}, not {}",
            2.0 * share
        );

        for _ in 0..buffers(180.0) {
            tilt.update(&line(2.0), true, BUFFER_RATE);
        }
        assert!(
            (tilt.slope() - 2.0).abs() < 0.01,
            "the tilt settled at {}, not 2",
            tilt.slope()
        );

        for _ in 0..buffers(180.0) {
            tilt.update(&line(5.0), true, BUFFER_RATE);
        }
        assert_eq!(tilt.slope(), Tilt::LIMIT, "a steep tilt stops at the limit");

        for _ in 0..buffers(60.0) {
            tilt.update(&line(-5.0), false, BUFFER_RATE);
        }
        assert_eq!(tilt.slope(), Tilt::LIMIT, "silence moved the tilt");

        let mut none_live = Tilt::new([false; NUM_BANDS], &octave);
        assert_eq!(
            none_live.update(&line(2.0), true, BUFFER_RATE),
            0.0,
            "a tilt over no bands"
        );
    }
}
