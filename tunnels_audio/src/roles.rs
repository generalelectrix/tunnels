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

use crate::bank::NUM_BANDS;
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
    /// The output, falling away after each hit.
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

    fn new(rate: f32) -> Self {
        Self {
            envelope: peak_follower(0.004, rate),
            baseline: AsymmetricOnePole::new(tc_coeff(0.030, rate), tc_coeff(0.005, rate)),
            recent_peak: peak_follower(3.0, rate),
            phrase_peak: peak_follower(20.0, rate),
            release: peak_follower(0.080, rate),
        }
    }

    fn process(&mut self, band_peak: f32) -> f32 {
        let level = self.envelope.step(band_peak);
        let baseline = self.baseline.step(level);
        let recent = self.recent_peak.step(level);
        let phrase = self.phrase_peak.step(level);
        let rise = (level - baseline).max(0.0) / recent.max(1e-9);
        let gated = level > phrase * Self::RELATIVE_GATE && level > Self::ABSOLUTE_GATE;
        let hit = if gated {
            ((rise - Self::KNEE) / (Self::FULL - Self::KNEE)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.release.step(hit)
    }
}

/// Buffers of level history a region keeps per band.
const HISTORY: usize = 64;

/// The most bands a region holds.
const MAX_REGION: usize = 6;

/// How sharply a group of bands has just risen: the median, across the bands,
/// of each band's rise in dB above its own loudest over the preceding few
/// milliseconds. A noisy hit rises in every band at once; a note rises in a
/// few, so the median ignores it.
///
/// Fixed in size, so building one allocates nothing.
struct RiseRegion {
    bands: [usize; MAX_REGION],
    len: usize,
    envelopes: [AsymmetricOnePole; MAX_REGION],
    /// Each band's recent levels in dB, newest at `head`.
    history: [[f32; HISTORY]; MAX_REGION],
    head: usize,
    /// The span of the history a rise is measured from, in buffers ago.
    from_lag: usize,
    to_lag: usize,
}

impl RiseRegion {
    /// The window a rise is measured over: from this long ago...
    const FROM_SECS: f32 = 0.0133;
    /// ...to this long ago.
    const TO_SECS: f32 = 0.0053;

    fn new(bands: &[usize], live: impl Fn(usize) -> bool, rate: f32) -> Self {
        let mut region = Self {
            bands: [0; MAX_REGION],
            len: 0,
            envelopes: [peak_follower(0.004, rate); MAX_REGION],
            // History starts out loud, so nothing is a rise until the region
            // has heard enough to measure one.
            history: [[f32::MAX; HISTORY]; MAX_REGION],
            head: 0,
            from_lag: 0,
            to_lag: 0,
        };
        for &band in bands.iter().filter(|&&b| live(b)).take(MAX_REGION) {
            region.bands[region.len] = band;
            region.len += 1;
        }
        region.to_lag = ((Self::TO_SECS * rate).round() as usize).max(1);
        region.from_lag =
            ((Self::FROM_SECS * rate).round() as usize).clamp(region.to_lag, HISTORY - 1);
        region
    }

    /// The region's rise in dB, or nothing if none of its bands can be heard.
    fn process(&mut self, peaks: &[f32; NUM_BANDS]) -> Option<f32> {
        if self.len == 0 {
            return None;
        }
        self.head = (self.head + 1) % HISTORY;
        let mut rises = [0.0_f32; MAX_REGION];
        for i in 0..self.len {
            let level = 20.0 * (self.envelopes[i].step(peaks[self.bands[i]]) + 1e-9).log10();
            let earlier = (self.to_lag..=self.from_lag)
                .map(|lag| self.history[i][(self.head + HISTORY - lag) % HISTORY])
                .fold(f32::MIN, f32::max);
            self.history[i][self.head] = level;
            rises[i] = (level - earlier).max(0.0);
        }
        let rises = &mut rises[..self.len];
        rises.sort_unstable_by(f32::total_cmp);
        let n = rises.len();
        Some(if n % 2 == 1 {
            rises[n / 2]
        } else {
            0.5 * (rises[n / 2 - 1] + rises[n / 2])
        })
    }
}

/// Hits in the high end: a rise across the high bands that stands out from
/// any rise just below them, so a snare's crack counts for less and a sung
/// consonant's for nothing. Rises are measured in dB, which makes them the
/// same at any level, so the same gate as [`KickRole`]'s keeps a faint tick
/// over near-silence from counting.
struct HatsRole {
    high: RiseRegion,
    presence: RiseRegion,
    /// The high end's combined level, holding each peak for a few
    /// milliseconds, and its loudest over the last phrase.
    level: AsymmetricOnePole,
    phrase_peak: AsymmetricOnePole,
    release: AsymmetricOnePole,
}

impl HatsRole {
    /// A rise this many dB across the high bands is a full hit.
    const FULL_DB: f32 = 6.0;

    fn new(live: impl Fn(usize) -> bool + Copy, rate: f32) -> Self {
        Self {
            high: RiseRegion::new(HIGH_BANDS, live, rate),
            presence: RiseRegion::new(PRESENCE_BANDS, live, rate),
            level: peak_follower(0.004, rate),
            phrase_peak: peak_follower(20.0, rate),
            release: peak_follower(0.080, rate),
        }
    }

    fn process(&mut self, peaks: &[f32; NUM_BANDS]) -> f32 {
        let power: f32 = HIGH_BANDS.iter().map(|&b| peaks[b] * peaks[b]).sum();
        let level = self.level.step(power.sqrt());
        let phrase = self.phrase_peak.step(level);
        let gated = level > phrase * KickRole::RELATIVE_GATE && level > KickRole::ABSOLUTE_GATE;
        let presence = self.presence.process(peaks).unwrap_or(0.0) / Self::FULL_DB;
        let hit = match self.high.process(peaks) {
            Some(_) if !gated => 0.0,
            Some(high) => {
                let high = high / Self::FULL_DB;
                (high * (high - presence + 0.5).clamp(0.0, 1.0)).clamp(0.0, 1.0)
            }
            None => 0.0,
        };
        self.release.step(hit)
    }
}

/// The level of a group of bands: their combined envelope through the
/// operator's envelope follower and smoother, then normalized against its own
/// recent range.
struct LevelRole {
    bands: &'static [usize],
    envelope: AsymmetricOnePole,
    smoother: OnePoleSmoother,
    normalizer: AdaptiveNormalizer,
}

impl LevelRole {
    fn new(bands: &'static [usize], tuning: &NormalizerTuning) -> Self {
        Self {
            bands,
            envelope: AsymmetricOnePole::default(),
            smoother: OnePoleSmoother::default(),
            normalizer: AdaptiveNormalizer::new(tuning),
        }
    }

    fn process(
        &mut self,
        peaks: &[f32; NUM_BANDS],
        smooth_coeff: f32,
        norm: &NormalizerParams,
    ) -> f32 {
        let power: f32 = self.bands.iter().map(|&b| peaks[b] * peaks[b]).sum();
        let level = self.envelope.step(power.sqrt());
        let smoothed = self.smoother.update(smooth_coeff, level);
        self.normalizer.process(smoothed, norm)
    }

    fn stages(&self) -> BandStages {
        self.normalizer.stages(self.smoother.state())
    }
}

/// Every role, run once per buffer from the bank's band peaks.
pub(crate) struct Roles {
    kick: KickRole,
    bass: LevelRole,
    hats: HatsRole,
    shimmer: LevelRole,
    /// Buffers per second the hit roles' timing was built for.
    rate: f32,
    live: [bool; NUM_BANDS],
}

impl Roles {
    pub(crate) fn new(live: [bool; NUM_BANDS], tuning: &NormalizerTuning) -> Self {
        Self {
            kick: KickRole::new(0.0),
            bass: LevelRole::new(LOW_BANDS, tuning),
            hats: HatsRole::new(|b| live[b], 0.0),
            shimmer: LevelRole::new(HIGH_BANDS, tuning),
            rate: 0.0,
            live,
        }
    }

    /// Restart the level roles' normalizers with a new tuning.
    pub(crate) fn set_normalizer_tuning(&mut self, tuning: &NormalizerTuning) {
        self.bass.normalizer = AdaptiveNormalizer::new(tuning);
        self.shimmer.normalizer = AdaptiveNormalizer::new(tuning);
    }

    /// Set the level roles' envelope attack and release half-lives, in
    /// seconds, at `rate` buffers per second; and rebuild the hit roles if the
    /// rate has changed, since their timing is fixed in seconds.
    pub(crate) fn set_timing(&mut self, attack: f32, release: f32, rate: f32) {
        for level in [&mut self.bass, &mut self.shimmer] {
            level.envelope.rise = halflife_to_coeff(attack, rate);
            level.envelope.fall = halflife_to_coeff(release, rate);
        }
        if rate != self.rate {
            self.rate = rate;
            let live = self.live;
            self.kick = KickRole::new(rate);
            self.hats = HatsRole::new(move |b| live[b], rate);
        }
    }

    /// Every role's output for one buffer, in [`Role::ALL`] order.
    pub(crate) fn process(
        &mut self,
        peaks: &[f32; NUM_BANDS],
        smooth_coeff: f32,
        norm: &NormalizerParams,
    ) -> [f32; NUM_ROLES] {
        [
            self.kick.process(peaks[KICK_BAND]),
            self.bass.process(peaks, smooth_coeff, norm),
            self.hats.process(peaks),
            self.shimmer.process(peaks, smooth_coeff, norm),
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
