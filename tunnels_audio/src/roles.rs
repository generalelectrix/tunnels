//! The four envelopes the show can follow, derived from the resonator bank.
//!
//! They split the spectrum's two ends by what they track: hits (fast,
//! level-independent onsets) and level (the end's loudness, normalized
//! against its own recent range).
//!
//! | | hits | level |
//! |---|---|---|
//! | low end | [`Role::Kick`] | [`Role::Bass`] |
//! | high end | [`Role::Hats`] | [`Role::Shimmer`] |

use crate::bank::{NUM_BANDS, ResonatorBank};
use crate::processor::{
    AdaptiveNormalizer, AsymmetricOnePole, BandStages, NormalizerParams, NormalizerTuning,
    OnePoleSmoother, halflife_to_coeff,
};

/// An envelope the show can follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Hits in the low end: kick drums, and any sharp low onset.
    Kick,
    /// The low end's level, kick included.
    Bass,
    /// Hits in the high end: hats, shakers, snare wires, cymbal strikes.
    Hats,
    /// The high end's level: cymbals, hats, sibilance and air.
    Shimmer,
}

/// Number of roles.
pub const NUM_ROLES: usize = 4;

impl Role {
    /// Every role, in output order.
    pub const ALL: [Self; NUM_ROLES] = [Self::Kick, Self::Bass, Self::Hats, Self::Shimmer];

    /// The role at an output index, if there is one.
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The role's position in the output order.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The role's name as an operator reads it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Bass => "Bass",
            Self::Hats => "Hats",
            Self::Shimmer => "Shimmer",
        }
    }
}

/// Bands the low end is heard in: about 50 to 250 Hz.
const LOW_BANDS: &[usize] = &[0, 1];
/// The band kicks are found in: about 50 to 100 Hz, below most bass notes'
/// fundamentals.
const KICK_BAND: usize = 0;
/// Bands the high end is heard in: about 6 to 16 kHz.
const HIGH_BANDS: &[usize] = &[20, 21, 22, 23, 24, 25];
/// Bands just under the high end, about 3 to 6 kHz, against which a hit has to
/// stand out to count as one in the highs.
const PRESENCE_BANDS: &[usize] = &[16, 17, 18, 19];

/// Coefficient of a one-pole follower with time constant `secs` at `rate`
/// updates per second; zero follows at once.
fn tc_coeff(secs: f32, rate: f32) -> f32 {
    if secs <= 0.0 || rate <= 0.0 {
        0.0
    } else {
        (-1.0 / (secs * rate)).exp()
    }
}

/// A follower that jumps to any rise and falls with time constant `secs`.
fn peak_follower(secs: f32, rate: f32) -> AsymmetricOnePole {
    AsymmetricOnePole::new(0.0, tc_coeff(secs, rate))
}

/// Hits in one band: how far its envelope stands above where it has just
/// been, as a fraction of how loud the band has recently been, so a hit
/// reads the same at any level.
///
/// Traced at the audio rate, sample by sample, and reported per buffer as
/// the largest hit the buffer held, so a hit reads the same wherever it
/// falls relative to the buffer grid.
struct KickRole {
    /// The band's envelope, holding each peak for a few milliseconds.
    envelope: AsymmetricOnePole,
    /// Where the envelope has just been: slow to rise, quick to fall, so a
    /// sudden rise leaves it behind.
    baseline: AsymmetricOnePole,
    /// The band's recent loudest, which a rise is measured against.
    recent_peak: AsymmetricOnePole,
    /// The band's loudest over the last phrase, for gating out noise once the
    /// music has stopped.
    phrase_peak: AsymmetricOnePole,
    /// The largest hit since the last buffer was reported.
    buffer_hit: f32,
    /// The output, falling away after each hit, at the buffer rate.
    release: AsymmetricOnePole,
}

impl KickRole {
    /// A rise below this fraction of the recent peak is the band's own
    /// wobble (two low notes beating, say) rather than a hit.
    const KNEE: f32 = 0.25;
    /// A rise of this fraction of the recent peak or more is a full hit.
    const FULL: f32 = 0.5;
    /// Hits are ignored this far below the phrase's peak.
    const RELATIVE_GATE: f32 = 0.01;
    /// And below this absolute level, where there is nothing but noise.
    const ABSOLUTE_GATE: f32 = 0.001;

    /// The release follower is replaced once the buffer rate is known.
    const RELEASE_SECS: f32 = 0.080;

    fn new(sample_rate: f32) -> Self {
        Self {
            envelope: peak_follower(0.004, sample_rate),
            baseline: AsymmetricOnePole::new(
                tc_coeff(0.030, sample_rate),
                tc_coeff(0.005, sample_rate),
            ),
            recent_peak: peak_follower(3.0, sample_rate),
            phrase_peak: peak_follower(20.0, sample_rate),
            buffer_hit: 0.0,
            release: peak_follower(Self::RELEASE_SECS, 0.0),
        }
    }

    /// Set the buffer rate the output's release runs at.
    fn set_buffer_rate(&mut self, rate: f32) {
        self.release.fall = tc_coeff(Self::RELEASE_SECS, rate);
    }

    /// Follow the band's envelope at one sample.
    #[inline]
    fn push(&mut self, magnitude: f32) {
        let level = self.envelope.step(magnitude);
        let baseline = self.baseline.step(level);
        let recent = self.recent_peak.step(level);
        let phrase = self.phrase_peak.step(level);
        if level > phrase * Self::RELATIVE_GATE && level > Self::ABSOLUTE_GATE {
            let rise = (level - baseline).max(0.0) / recent.max(1e-9);
            let hit = ((rise - Self::KNEE) / (Self::FULL - Self::KNEE)).clamp(0.0, 1.0);
            self.buffer_hit = self.buffer_hit.max(hit);
        }
    }

    /// The output for the buffer just ended.
    fn finish(&mut self) -> f32 {
        let hit = std::mem::take(&mut self.buffer_hit);
        self.release.step(hit)
    }
}

/// The highest level over a window that ends some time ago: at sample `n`,
/// the maximum of the levels from `from` samples ago to `to` samples ago,
/// exactly, at every sample.
///
/// A queue of the window's candidates for maximum, each larger than every
/// candidate after it, makes each sample's update amortized constant time.
/// Its storage is sized for the window when built, so following a signal
/// allocates nothing.
struct LaggedMax {
    /// The last `to` levels, so each enters the window `to` samples late.
    delay: Box<[f32]>,
    delay_head: usize,
    /// The candidates, as a ring of (sample index, level), oldest at `front`.
    queue: Box<[(u64, f32)]>,
    front: usize,
    len: usize,
    from: u64,
    to: u64,
    /// Samples followed so far.
    n: u64,
}

impl LaggedMax {
    fn new(from: usize, to: usize) -> Self {
        let to = to.max(1);
        let from = from.max(to);
        Self {
            delay: vec![0.0; to].into_boxed_slice(),
            delay_head: 0,
            queue: vec![(0, 0.0); from - to + 2].into_boxed_slice(),
            front: 0,
            len: 0,
            from: from as u64,
            to: to as u64,
            n: 0,
        }
    }

    /// Follow one more level, returning the window's maximum, or nothing until
    /// the window has filled.
    #[inline]
    fn push(&mut self, level: f32) -> Option<f32> {
        // The level `to` samples ago enters the window now.
        let entering = std::mem::replace(&mut self.delay[self.delay_head], level);
        self.delay_head = (self.delay_head + 1) % self.delay.len();
        let cap = self.queue.len();
        if self.n >= self.to {
            let index = self.n - self.to;
            while self.len > 0 && self.queue[(self.front + self.len - 1) % cap].1 <= entering {
                self.len -= 1;
            }
            self.queue[(self.front + self.len) % cap] = (index, entering);
            self.len += 1;
        }
        // Candidates from more than `from` samples ago have left it.
        while self.len > 0 && self.queue[self.front].0 + self.from < self.n {
            self.front = (self.front + 1) % cap;
            self.len -= 1;
        }
        self.n += 1;
        (self.n > self.from && self.len > 0).then(|| self.queue[self.front].1)
    }
}

/// The most bands a region holds.
const MAX_REGION: usize = 6;

/// How sharply a group of bands has just risen, traced sample by sample: the
/// median, across the bands, of each band's rise in dB above its own loudest
/// over the preceding few milliseconds. A noisy hit rises in every band at
/// once; a note rises in a few, so the median ignores it.
struct RiseRegion {
    bands: [usize; MAX_REGION],
    len: usize,
    envelopes: [AsymmetricOnePole; MAX_REGION],
    /// Each band's loudest over the look-back.
    earlier: [LaggedMax; MAX_REGION],
}

impl RiseRegion {
    /// The look-back a rise is measured from: from this long ago...
    const FROM_SECS: f32 = 0.0133;
    /// ...to this long ago.
    const TO_SECS: f32 = 0.0053;

    fn new(bands: &[usize], live: impl Fn(usize) -> bool, sample_rate: f32) -> Self {
        let from = (Self::FROM_SECS * sample_rate).round() as usize;
        let to = (Self::TO_SECS * sample_rate).round() as usize;
        let mut region = Self {
            bands: [0; MAX_REGION],
            len: 0,
            envelopes: [peak_follower(0.004, sample_rate); MAX_REGION],
            earlier: std::array::from_fn(|_| LaggedMax::new(from, to)),
        };
        for &band in bands.iter().filter(|&&b| live(b)).take(MAX_REGION) {
            region.bands[region.len] = band;
            region.len += 1;
        }
        region
    }

    /// The region's rise in dB at this sample, or nothing if none of its bands
    /// can be heard.
    #[inline]
    fn push(&mut self, bank: &ResonatorBank) -> Option<f32> {
        if self.len == 0 {
            return None;
        }
        // A rise in dB is the log of a ratio of levels, and the log keeps
        // their order: the median rise is the log of the median ratio, taken
        // once rather than per band. Until the look-back has filled, nothing
        // is a rise.
        let mut ratios = [1.0_f32; MAX_REGION];
        for (((&band, envelope), earlier), ratio) in self.bands[..self.len]
            .iter()
            .zip(&mut self.envelopes)
            .zip(&mut self.earlier)
            .zip(&mut ratios)
        {
            let level = envelope.step(bank.magnitude(band));
            if let Some(earlier) = earlier.push(level) {
                *ratio = (level / earlier.max(1e-12)).max(1.0);
            }
        }
        let ratios = &mut ratios[..self.len];
        ratios.sort_unstable_by(f32::total_cmp);
        let n = ratios.len();
        let median = if n % 2 == 1 {
            ratios[n / 2]
        } else {
            (ratios[n / 2 - 1] * ratios[n / 2]).sqrt()
        };
        Some(20.0 * median.log10())
    }
}

/// Hits in the high end: a rise across the high bands that stands out from
/// any rise just below them, so a snare's crack counts for less and a sung
/// consonant's for nothing. Rises are measured in dB, which makes them the
/// same at any level, so the same gate as [`KickRole`]'s keeps a faint tick
/// over near-silence from counting.
///
/// Traced sample by sample and reported per buffer as the largest hit the
/// buffer held, like [`KickRole`]: the two regions respond at different
/// speeds, and comparing them once a buffer would make a hit's reading
/// depend on where it fell against the buffers.
struct HatsRole {
    high: RiseRegion,
    presence: RiseRegion,
    /// The high end's combined level, holding each peak for a few
    /// milliseconds, and its loudest over the last phrase.
    level: AsymmetricOnePole,
    phrase_peak: AsymmetricOnePole,
    /// The largest hit since the last buffer was reported.
    buffer_hit: f32,
    /// The output, falling away after each hit, at the buffer rate.
    release: AsymmetricOnePole,
}

impl HatsRole {
    /// A rise this many dB across the high bands is a full hit.
    const FULL_DB: f32 = 6.0;
    const RELEASE_SECS: f32 = 0.080;

    fn new(live: impl Fn(usize) -> bool + Copy, sample_rate: f32) -> Self {
        Self {
            high: RiseRegion::new(HIGH_BANDS, live, sample_rate),
            presence: RiseRegion::new(PRESENCE_BANDS, live, sample_rate),
            level: peak_follower(0.004, sample_rate),
            phrase_peak: peak_follower(20.0, sample_rate),
            buffer_hit: 0.0,
            release: peak_follower(Self::RELEASE_SECS, 0.0),
        }
    }

    /// Set the buffer rate the output's release runs at.
    fn set_buffer_rate(&mut self, rate: f32) {
        self.release.fall = tc_coeff(Self::RELEASE_SECS, rate);
    }

    /// Follow the high end at one sample.
    #[inline]
    fn push(&mut self, bank: &ResonatorBank) {
        let power: f32 = HIGH_BANDS
            .iter()
            .map(|&b| {
                let m = bank.magnitude(b);
                m * m
            })
            .sum();
        let level = self.level.step(power.sqrt());
        let phrase = self.phrase_peak.step(level);
        let presence = self.presence.push(bank).unwrap_or(0.0) / Self::FULL_DB;
        let Some(high) = self.high.push(bank) else {
            return;
        };
        if level > phrase * KickRole::RELATIVE_GATE && level > KickRole::ABSOLUTE_GATE {
            let high = high / Self::FULL_DB;
            let hit = (high * (high - presence + 0.5).clamp(0.0, 1.0)).clamp(0.0, 1.0);
            self.buffer_hit = self.buffer_hit.max(hit);
        }
    }

    /// The output for the buffer just ended.
    fn finish(&mut self) -> f32 {
        let hit = std::mem::take(&mut self.buffer_hit);
        self.release.step(hit)
    }
}

/// The level of a group of bands: their combined envelope through the
/// operator's envelope follower and smoother, then normalized against its own
/// recent range.
///
/// The envelope follower runs sample by sample and is read at the end of
/// each buffer, so the level does not depend on where the music falls against
/// the buffers; the smoother and normalizer run once per buffer.
struct LevelRole {
    bands: &'static [usize],
    envelope: AsymmetricOnePole,
    /// The envelope at the latest sample.
    level: f32,
    smoother: OnePoleSmoother,
    normalizer: AdaptiveNormalizer,
}

impl LevelRole {
    fn new(bands: &'static [usize], tuning: &NormalizerTuning) -> Self {
        Self {
            bands,
            envelope: AsymmetricOnePole::default(),
            level: 0.0,
            smoother: OnePoleSmoother::default(),
            normalizer: AdaptiveNormalizer::new(tuning),
        }
    }

    /// Follow the bands' combined envelope at one sample.
    #[inline]
    fn push(&mut self, bank: &ResonatorBank) {
        let power: f32 = self
            .bands
            .iter()
            .map(|&b| {
                let m = bank.magnitude(b);
                m * m
            })
            .sum();
        self.level = self.envelope.step(power.sqrt());
    }

    /// The output for the buffer just ended.
    fn finish(&mut self, smooth_coeff: f32, norm: &NormalizerParams) -> f32 {
        let smoothed = self.smoother.update(smooth_coeff, self.level);
        self.normalizer.process(smoothed, norm)
    }

    fn stages(&self) -> BandStages {
        self.normalizer.stages(self.smoother.state())
    }
}

/// Every role, following the bank sample by sample and reported once per
/// buffer.
pub(crate) struct Roles {
    kick: KickRole,
    bass: LevelRole,
    hats: HatsRole,
    shimmer: LevelRole,
    sample_rate: f32,
    /// Buffers per second the hit roles' release was last set for.
    rate: f32,
}

impl Roles {
    pub(crate) fn new(
        live: [bool; NUM_BANDS],
        tuning: &NormalizerTuning,
        sample_rate: f32,
    ) -> Self {
        Self {
            kick: KickRole::new(sample_rate),
            bass: LevelRole::new(LOW_BANDS, tuning),
            hats: HatsRole::new(|b| live[b], sample_rate),
            shimmer: LevelRole::new(HIGH_BANDS, tuning),
            sample_rate,
            rate: 0.0,
        }
    }

    /// Restart the level roles' normalizers with a new tuning.
    pub(crate) fn set_normalizer_tuning(&mut self, tuning: &NormalizerTuning) {
        self.bass.normalizer = AdaptiveNormalizer::new(tuning);
        self.shimmer.normalizer = AdaptiveNormalizer::new(tuning);
    }

    /// Set the level roles' envelope attack and release half-lives, in
    /// seconds, and the buffer rate the hit roles' release runs at.
    pub(crate) fn set_timing(&mut self, attack: f32, release: f32, rate: f32) {
        for level in [&mut self.bass, &mut self.shimmer] {
            level.envelope.rise = halflife_to_coeff(attack, self.sample_rate);
            level.envelope.fall = halflife_to_coeff(release, self.sample_rate);
        }
        if rate != self.rate {
            self.rate = rate;
            self.kick.set_buffer_rate(rate);
            self.hats.set_buffer_rate(rate);
        }
    }

    /// Follow the bank at one sample.
    #[inline]
    pub(crate) fn push_sample(&mut self, bank: &ResonatorBank) {
        self.kick.push(bank.magnitude(KICK_BAND));
        self.bass.push(bank);
        self.hats.push(bank);
        self.shimmer.push(bank);
    }

    /// Every role's output for one buffer, in [`Role::ALL`] order.
    pub(crate) fn finish(
        &mut self,
        smooth_coeff: f32,
        norm: &NormalizerParams,
    ) -> [f32; NUM_ROLES] {
        [
            self.kick.finish(),
            self.bass.finish(smooth_coeff, norm),
            self.hats.finish(),
            self.shimmer.finish(smooth_coeff, norm),
        ]
    }

    /// A level role's intermediate stages; hit roles have none.
    pub(crate) fn stages(&self, role: Role) -> Option<BandStages> {
        match role {
            Role::Bass => Some(self.bass.stages()),
            Role::Shimmer => Some(self.shimmer.stages()),
            Role::Kick | Role::Hats => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::{Processor, ProcessorSettings, envelope_ring_buffers};

    const RATE: f32 = 48_000.0;
    const FRAMES: usize = 64;

    /// Every role's output per 64-frame buffer, for a mono signal.
    fn run(signal: &[f32]) -> Vec<[f32; NUM_ROLES]> {
        let buffers = envelope_ring_buffers();
        let mut streams = buffers.streams;
        let mut processor = Processor::new(
            ProcessorSettings::default(),
            RATE as u32,
            1,
            buffers.producers,
        );
        let mut drained = Vec::new();
        signal
            .as_chunks::<FRAMES>()
            .0
            .iter()
            .map(|chunk| {
                processor.process(chunk);
                std::array::from_fn(|r| {
                    drained.clear();
                    streams[r].drain_into(&mut drained);
                    drained.last().copied().unwrap_or(0.0)
                })
            })
            .collect()
    }

    /// The buffer a time falls in.
    fn at(secs: f32) -> usize {
        (secs * RATE) as usize / FRAMES
    }

    /// The largest output of `role` over a span of buffers.
    fn max(out: &[[f32; NUM_ROLES]], role: Role, from: usize, to: usize) -> f32 {
        out[from..to]
            .iter()
            .map(|o| o[role.index()])
            .fold(0.0, f32::max)
    }

    /// White noise from a fixed seed, pushed toward the treble by three first
    /// differences (+18 dB/octave), so it is all highs and no low end.
    fn bright_noise(n: usize) -> Vec<f32> {
        let mut seed = 0x2545_f491_u32;
        let mut noise: Vec<f32> = (0..n)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as f32 / u32::MAX as f32 - 0.5
            })
            .collect();
        for _ in 0..3 {
            for i in (1..n).rev() {
                noise[i] -= noise[i - 1];
            }
        }
        noise
    }

    /// The lagged maximum is exactly the brute-force maximum over its window
    /// at every sample once the window has filled, and nothing before.
    #[test]
    fn lagged_max_is_exact() {
        let (from, to) = (37, 11);
        let mut lagged = LaggedMax::new(from, to);
        let mut seed = 0x9e37_79b9_u32;
        let mut levels = Vec::new();
        for n in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            // Runs of repeats test ties as well as ordinary values.
            let level = (seed % 17) as f32;
            levels.push(level);
            let got = lagged.push(level);
            if n < from {
                assert_eq!(got, None, "sample {n}: the window has not filled");
            } else {
                let want = levels[n - from..=n - to]
                    .iter()
                    .copied()
                    .fold(f32::MIN, f32::max);
                assert_eq!(got, Some(want), "sample {n}");
            }
        }
    }

    /// Each role answers what it is for and stays quiet for what it is not:
    /// low thumps are Kick and not Hats, bright ticks are Hats and not Kick,
    /// and a held low or high tone is Bass or Shimmer, not the other.
    #[test]
    fn each_role_hears_its_own_end_of_the_spectrum() {
        let n = (8.0 * RATE) as usize;
        let t = |i: usize| i as f32 / RATE;

        // A decaying 55 Hz thump every half second, faded in over 3 ms and
        // gone well before the next, so it carries no click: all low end.
        let kicks: Vec<f32> = (0..n)
            .map(|i| {
                let since = t(i) % 0.5;
                let fade_in = (since / 0.003).min(1.0);
                0.8 * fade_in * (-since / 0.05).exp() * (std::f32::consts::TAU * 55.0 * since).sin()
            })
            .collect();
        let out = run(&kicks);
        for hit in 8..16 {
            let on = hit as f32 * 0.5;
            let peak = max(&out, Role::Kick, at(on), at(on + 0.1));
            let before = out[at(on) - 1][Role::Kick.index()];
            assert!(peak > 0.8, "kick {hit} reads {peak}");
            assert!(
                before < 0.1,
                "Kick has not let go before kick {hit}: {before}"
            );
        }
        assert!(
            max(&out, Role::Hats, at(4.0), at(8.0)) < 0.1,
            "thumps are not hats"
        );

        // A 10 ms bright tick every quarter second, shaped by a Hann window: a
        // tick switched on and off abruptly would be a click, which reaches
        // the low end too.
        let noise = bright_noise(n);
        let ticks: Vec<f32> = (0..n)
            .map(|i| {
                let since = t(i) % 0.25;
                if since < 0.01 {
                    let hann = 0.5 - 0.5 * (std::f32::consts::TAU * since / 0.01).cos();
                    0.5 * hann * noise[i]
                } else {
                    0.0
                }
            })
            .collect();
        let out = run(&ticks);
        for tick in 16..32 {
            let on = tick as f32 * 0.25;
            let peak = max(&out, Role::Hats, at(on), at(on + 0.05));
            assert!(peak > 0.8, "tick {tick} reads {peak}");
        }
        assert!(
            max(&out, Role::Kick, at(4.0), at(8.0)) < 0.1,
            "ticks are not kicks"
        );

        // Two seconds of a held 80 Hz tone, then two of a held 10 kHz tone.
        let tones: Vec<f32> = (0..(4.0 * RATE) as usize)
            .map(|i| {
                let f = if t(i) < 2.0 { 80.0 } else { 10_000.0 };
                0.5 * (std::f32::consts::TAU * f * t(i)).sin()
            })
            .collect();
        let out = run(&tones);
        let low = &out[at(1.0)..at(2.0)];
        let high = &out[at(3.0)..at(4.0)];
        let mean = |span: &[[f32; NUM_ROLES]], role: Role| {
            span.iter().map(|o| o[role.index()]).sum::<f32>() / span.len() as f32
        };
        assert!(
            mean(low, Role::Bass) > 0.5,
            "Bass during the low tone: {}",
            mean(low, Role::Bass)
        );
        assert!(
            mean(low, Role::Shimmer) < 0.05,
            "Shimmer during the low tone: {}",
            mean(low, Role::Shimmer)
        );
        assert!(
            mean(high, Role::Shimmer) > 0.5,
            "Shimmer during the high tone: {}",
            mean(high, Role::Shimmer)
        );
        assert!(
            mean(high, Role::Bass) < 0.05,
            "Bass during the high tone: {}",
            mean(high, Role::Bass)
        );
    }
}
