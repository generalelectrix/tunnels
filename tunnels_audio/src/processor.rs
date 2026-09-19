//! A multi-channel audio processor that derives per-band envelopes from its input.
//!
//! Processing chains:
//! Both run on the mono mix of the input channels:
//!   Lowpass: lowpass → Hilbert |z(t)| → fast envelope → slow envelope
//!   Wavelet: undecimated D4 decomposition → per-band Hilbert → fast → slow envelope
//!
//! Output: 8 normalized bands (1 lowpass + 7 wavelet), selectable via `active_band`.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{AtomicF32, AudioContext, simple_processor::MonoAudioProcessor};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::hilbert::HilbertTransform;
use crate::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};
use crate::wavelet::{NUM_LEVELS, WaveletDecomposition};

/// Fast envelope follower: catches every peak within a cycle.
const FAST_ATTACK: Duration = Duration::from_millis(1);
const FAST_RELEASE: Duration = Duration::new(0, 4_000_000); // 4ms

/// Number of output bands: 1 lowpass sub-bass + 7 wavelet bands.
pub const NUM_OUTPUT_BANDS: usize = 8;

/// Ring buffer capacity: ~16 seconds of history at ~1kHz buffer rate.
pub const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// Band labels in frequency-ascending output order (index 0 = lowpass sub-bass).
pub use crate::wavelet::BAND_LABELS as OUTPUT_BAND_LABELS;

/// The envelope ring buffers for every output band: the producers feed a
/// `Processor`, the streams are read by whoever displays or records them.
pub struct EnvelopeRingBuffers {
    pub producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],
    pub streams: [EnvelopeStream; NUM_OUTPUT_BANDS],
}

/// Create one envelope ring buffer per output band, each holding
/// `ENVELOPE_HISTORY_CAPACITY` values.
pub fn envelope_ring_buffers() -> EnvelopeRingBuffers {
    let (producers, streams): (Vec<_>, Vec<_>) = (0..NUM_OUTPUT_BANDS)
        .map(|_| envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY))
        .unzip();
    EnvelopeRingBuffers {
        producers: producers.try_into().ok().expect("one producer per band"),
        streams: streams.try_into().ok().expect("one stream per band"),
    }
}

/// Audio callback rate in Hz (sample_rate / frames_per_buffer).
#[derive(Debug, Clone, Copy)]
pub struct UpdateRate(f32);

impl UpdateRate {
    pub fn new(sample_rate: u32, frames_per_buffer: u32) -> Self {
        Self(sample_rate as f32 / frames_per_buffer as f32)
    }

    pub fn as_hz(self) -> f32 {
        self.0
    }

    pub fn interval_secs(self) -> f64 {
        1.0 / self.0 as f64
    }
}

pub struct ProcessorSettingsInner {
    /// Current envelope value for the show loop (from active_band).
    pub envelope: AtomicF32,
    pub filter_cutoff: AtomicF32,    // Hz
    pub envelope_attack: AtomicF32,  // sec (slow stage)
    pub envelope_release: AtomicF32, // sec (slow stage)
    /// Input signal gain multiplier (linear scale).
    pub gain: AtomicF32,
    /// Symmetric output smoothing time constant (seconds). 0 = disabled.
    pub output_smoothing: AtomicF32,
    /// Whether the auto-trim is enabled.
    pub auto_trim_enabled: AtomicBool,
    /// Current auto-trim gain factor (read by GUI for display).
    pub auto_trim_gain: AtomicF32,

    /// Floor tracking half-life in seconds (slow — adapts to ambient level).
    pub norm_floor_halflife: AtomicF32,
    /// Ceiling tracking half-life in seconds (moderate — tracks recent peaks).
    pub norm_ceiling_halflife: AtomicF32,
    pub norm_floor_mode: AtomicTrackingMode,
    pub norm_ceiling_mode: AtomicTrackingMode,

    /// Which band feeds `envelope`: 0 = lowpass, 1-7 = wavelet bands.
    pub active_band: AtomicU32,
}

impl ProcessorSettingsInner {
    const DEFAULT_FILTER_CUTOFF: f32 = 187.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;

    pub fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
        self.gain.set(1.0);
        self.auto_trim_enabled.store(true, Ordering::Relaxed);
        self.active_band.store(0, Ordering::Relaxed);
        self.norm_floor_halflife.set(10.0);
        self.norm_ceiling_halflife.set(5.0);
        self.norm_floor_mode
            .store(TrackingMode::Average, Ordering::Relaxed);
        self.norm_ceiling_mode
            .store(TrackingMode::Limit, Ordering::Relaxed);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            filter_cutoff: AtomicF32::new(Self::DEFAULT_FILTER_CUTOFF),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
            gain: AtomicF32::new(1.0),
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            auto_trim_enabled: AtomicBool::new(true),
            auto_trim_gain: AtomicF32::new(1.0),
            norm_floor_halflife: AtomicF32::new(10.0),
            norm_ceiling_halflife: AtomicF32::new(5.0),
            norm_floor_mode: AtomicTrackingMode::new(TrackingMode::Average),
            norm_ceiling_mode: AtomicTrackingMode::new(TrackingMode::Limit),
            active_band: AtomicU32::new(0),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// Input gain trim: slow automatic gain that compensates for gradual drift
/// in the feed level. NOT compression — just keeping the pipe full.
///
/// Slews in dB (log) space so that equal perceptual changes (+6 dB vs -6 dB)
/// take equal time at the same coefficient, independent of the current gain.
/// All time constants are in seconds and derived from the update rate.
struct AutoTrim {
    /// Tracked peak level: instant attack, slow decay.
    peak_tracker: f32,
    /// Current trim gain in dB.
    gain_db: f32,
    /// Current trim gain as a linear multiplier (cached from gain_db).
    gain: f32,
    /// Update rate the cached coefficients were derived for.
    update_rate: f32,
    peak_fall_coeff: f32,
    gain_coeff: f32,
}

impl AutoTrim {
    /// Target peak level. Unity — downstream is all floating point, and a
    /// momentary overshoot is harmless.
    const TARGET: f32 = 1.0;
    /// Gain range in dB.
    const MIN_GAIN_DB: f32 = -10.0;
    const MAX_GAIN_DB: f32 = 10.0;
    /// Peak tracker fall half-life.
    const PEAK_FALL_HALFLIFE: f32 = 10.0;
    /// Gain slew half-life, the same in both directions. Overshoot is
    /// harmless, so there is nothing to race toward, and the normalizers
    /// downstream follow a slow change of level transparently, so a stray
    /// peak that pulls the tracker up costs only a gentle, brief dip.
    const GAIN_HALFLIFE: f32 = 5.0;
    /// Buffer peak below which the trim holds still, so silence and idle
    /// noise are never boosted toward the target.
    const SILENCE: f32 = 0.01;

    fn new() -> Self {
        Self {
            peak_tracker: 0.0,
            gain_db: 0.0,
            gain: 1.0,
            update_rate: 0.0,
            peak_fall_coeff: 0.0,
            gain_coeff: 0.0,
        }
    }

    fn db_to_linear(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    fn linear_to_db(lin: f32) -> f32 {
        20.0 * lin.log10()
    }

    /// Refresh the cached coefficients for the update rate. Safe to call
    /// every buffer.
    fn set_params(&mut self, update_rate: f32) {
        if update_rate == self.update_rate {
            return;
        }
        self.update_rate = update_rate;
        self.peak_fall_coeff = halflife_to_coeff(Self::PEAK_FALL_HALFLIFE, update_rate);
        self.gain_coeff = halflife_to_coeff(Self::GAIN_HALFLIFE, update_rate);
    }

    /// Update the trim from the peak level observed in this buffer.
    fn update(&mut self, buffer_peak: f32) {
        if buffer_peak < Self::SILENCE {
            return;
        }

        if buffer_peak > self.peak_tracker {
            self.peak_tracker = buffer_peak;
        } else {
            self.peak_tracker = self.peak_fall_coeff * self.peak_tracker
                + (1.0 - self.peak_fall_coeff) * buffer_peak;
        }

        let desired_db = Self::linear_to_db(Self::TARGET / self.peak_tracker)
            .clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
        self.gain_db = self.gain_coeff * self.gain_db + (1.0 - self.gain_coeff) * desired_db;
        self.gain = Self::db_to_linear(self.gain_db);
    }
}

/// One-pole EMA coefficient that halves the distance to the target every
/// `halflife_secs` at `update_rate` updates per second. A non-positive
/// half-life means no smoothing.
fn halflife_to_coeff(halflife_secs: f32, update_rate: f32) -> f32 {
    if halflife_secs <= 0.0 || update_rate <= 0.0 {
        return 0.0;
    }
    (-f32::ln(2.0) / (halflife_secs * update_rate)).exp()
}

/// Tracking mode for floor/ceiling.
/// - Average: asymmetric EMA tracking the general level
/// - Limit: tracks the instantaneous min (floor) or max (ceiling) with slow decay
#[derive(PartialEq, Eq)]
#[atomic_enum::atomic_enum]
pub enum TrackingMode {
    Average = 0,
    Limit = 1,
}

/// The normalizer coefficients every band shares, derived from the settings
/// and the buffer update rate. The expensive `exp()`s are recomputed only
/// when a half-life or the update rate changes.
struct NormalizerParams {
    floor_mode: TrackingMode,
    ceiling_mode: TrackingMode,
    /// EMA coefficients for average-mode floor tracking.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// EMA coefficient for limit-mode floor (slow rise from minimum).
    floor_limit_rise_coeff: f32,
    /// EMA coefficient for ceiling decay.
    ceiling_fall_coeff: f32,
    /// EMA coefficient for ceiling decay while nothing approaches it.
    ceiling_fast_fall_coeff: f32,
    /// EMA coefficient for the ceiling anchor's rise toward the ceiling.
    anchor_rise_coeff: f32,
    /// Seconds per update.
    interval: f64,
    /// The inputs the coefficients were derived from.
    floor_halflife: f32,
    ceiling_halflife: f32,
    update_rate: f32,
}

impl NormalizerParams {
    fn new() -> Self {
        Self {
            floor_mode: TrackingMode::Average,
            ceiling_mode: TrackingMode::Limit,
            floor_rise_coeff: 0.0,
            floor_fall_coeff: 0.0,
            floor_limit_rise_coeff: 0.0,
            ceiling_fall_coeff: 0.0,
            ceiling_fast_fall_coeff: 0.0,
            anchor_rise_coeff: 0.0,
            interval: 0.0,
            floor_halflife: 0.0,
            ceiling_halflife: 0.0,
            update_rate: 0.0,
        }
    }

    /// Refresh from the settings for the given update rate. Safe to call
    /// every buffer.
    fn refresh(&mut self, settings: &ProcessorSettingsInner, update_rate: f32) {
        self.floor_mode = settings.norm_floor_mode.load(Ordering::Relaxed);
        self.ceiling_mode = settings.norm_ceiling_mode.load(Ordering::Relaxed);
        let floor_halflife = settings.norm_floor_halflife.get();
        let ceiling_halflife = settings.norm_ceiling_halflife.get();
        if update_rate <= 0.0
            || (floor_halflife == self.floor_halflife
                && ceiling_halflife == self.ceiling_halflife
                && update_rate == self.update_rate)
        {
            return;
        }
        self.floor_halflife = floor_halflife;
        self.ceiling_halflife = ceiling_halflife;
        self.update_rate = update_rate;
        // Average mode: slow rise, faster fall.
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff = halflife_to_coeff(floor_halflife * 0.2, update_rate);
        // Limit mode: instant drop to min, slow rise back.
        self.floor_limit_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        // Ceiling: instant (bounded) attack, decays at ceiling halflife.
        self.ceiling_fall_coeff = halflife_to_coeff(ceiling_halflife, update_rate);
        self.ceiling_fast_fall_coeff =
            halflife_to_coeff(AdaptiveNormalizer::CEILING_FAST_FALL_HALFLIFE, update_rate);
        self.anchor_rise_coeff =
            halflife_to_coeff(AdaptiveNormalizer::CEILING_ANCHOR_HALFLIFE, update_rate);
        self.interval = 1.0 / f64::from(update_rate);
    }
}

/// Detects a change of level: distinct overshoots well above a reference,
/// either several in quick succession or one that is sustained.
struct OvershootDetector {
    /// Start times of the most recent overshoots, oldest first.
    starts: [f64; Self::CONFIRM_COUNT],
    /// Whether the envelope is currently in an overshoot.
    active: bool,
    /// Peak envelope of the current overshoot; the overshoot ends once the
    /// envelope falls below `OVERSHOOT_END` times this.
    peak: f32,
    /// Envelope on the previous update, so an overshoot only begins on a
    /// rising envelope: the decaying tail of a spike never re-triggers.
    prev_envelope: f32,
}

impl OvershootDetector {
    /// An overshoot is an excursion this far above the reference. Ordinary
    /// beat-to-beat variation in peak height stays well inside this, so only
    /// a real jump in level counts.
    const OVERSHOOT_MARGIN: f32 = 1.5;
    /// An overshoot ends when the envelope falls to this fraction of its
    /// peak, so consecutive beats count separately even while the reference
    /// is still far below them.
    const OVERSHOOT_END: f32 = 0.5;
    /// This many distinct overshoots within `CONFIRM_WINDOW` seconds — or
    /// one overshoot lasting that long — confirm a change of level. One loud
    /// hit never confirms; a louder passage or a cold start confirms itself
    /// in a few beats, a sustained tone by outlasting the window.
    const CONFIRM_COUNT: usize = 3;
    const CONFIRM_WINDOW: f64 = 1.5;

    fn new() -> Self {
        Self {
            starts: [f64::NEG_INFINITY; Self::CONFIRM_COUNT],
            active: false,
            peak: 0.0,
            prev_envelope: 0.0,
        }
    }

    /// Feed one envelope value at time `now`; returns whether a change of
    /// level is confirmed.
    #[inline]
    fn update(&mut self, envelope: f32, reference: f32, now: f64) -> bool {
        if self.active {
            self.peak = self.peak.max(envelope);
            if envelope < Self::OVERSHOOT_END * self.peak {
                self.active = false;
            }
        } else if envelope > reference * Self::OVERSHOOT_MARGIN && envelope > self.prev_envelope {
            self.active = true;
            self.peak = envelope;
            self.starts.rotate_left(1);
            self.starts[Self::CONFIRM_COUNT - 1] = now;
        }
        self.prev_envelope = envelope;
        let latest = self.starts[Self::CONFIRM_COUNT - 1];
        now - self.starts[0] <= Self::CONFIRM_WINDOW
            || (self.active && now - latest > Self::CONFIRM_WINDOW)
    }
}

/// Adaptive envelope normalizer: tracks a floor and ceiling,
/// outputs `(envelope - floor) / (ceiling - floor)` clamped to [0, 1].
struct AdaptiveNormalizer {
    floor: f32,
    ceiling: f32,
    /// A lagged copy of the ceiling that bounds how far excursions above it
    /// can push it: in limit mode the ceiling rises to at most
    /// `CEILING_MAX_RISE` times this anchor. The anchor follows the ceiling
    /// up with `CEILING_ANCHOR_HALFLIFE` and down immediately, so excursions
    /// in quick succession share one allowance instead of compounding.
    ceiling_anchor: f32,
    overshoots: OvershootDetector,
    /// Elapsed processing time in seconds. Accumulated in f64: an f32 sum of
    /// millisecond steps loses the step itself after a few hours.
    now: f64,
    /// When the envelope last reached `CEILING_REACH` of the ceiling.
    last_reached: f64,
    /// Envelope level below which the band outputs zero: `NOISE_GATE` scaled
    /// by any fixed gain applied ahead of this band's envelope.
    gate: f32,
}

impl AdaptiveNormalizer {
    /// Minimum normalization range as a fraction of the ceiling. Once the
    /// floor has climbed to within this fraction of the ceiling the output
    /// fades instead of being stretched back to full scale, and a band's
    /// reach to full scale never depends on its absolute level.
    const REL_MIN_RANGE: f32 = 0.25;
    /// Input level below which the band outputs zero, so idle noise is
    /// never normalized up to full scale.
    const NOISE_GATE: f32 = 0.01;
    /// Most the ceiling can exceed its anchor. A lone loud hit or a click
    /// nudges the ceiling by this factor instead of setting it.
    const CEILING_MAX_RISE: f32 = 1.2;
    /// How quickly the anchor follows the ceiling up. Much longer than a
    /// kick, so a kick gets one nudge and a burst of them still only a few;
    /// a sustained excursion with no beats to confirm it compounds the nudge
    /// at this rate.
    const CEILING_ANCHOR_HALFLIFE: f32 = 1.0;
    /// The envelope "reaches" the ceiling when it comes within this fraction
    /// of it. During any beat-driven passage that happens every beat.
    const CEILING_REACH: f32 = 0.8;
    /// Once nothing has reached the ceiling for this long the level has
    /// dropped, and the ceiling releases at `CEILING_FAST_FALL_HALFLIFE`
    /// until something reaches it again.
    const CEILING_UNREACHED_SECS: f64 = 2.0;
    const CEILING_FAST_FALL_HALFLIFE: f32 = 0.5;

    /// `pre_gain` is the fixed gain applied to this band's signal ahead of
    /// envelope extraction, so the gate applies at input level.
    fn new(pre_gain: f32) -> Self {
        Self {
            floor: 0.0,
            ceiling: 0.001,
            ceiling_anchor: 0.001,
            overshoots: OvershootDetector::new(),
            now: 0.0,
            last_reached: 0.0,
            gate: Self::NOISE_GATE * pre_gain,
        }
    }

    #[inline]
    fn process(&mut self, envelope: f32, p: &NormalizerParams) -> f32 {
        self.now += p.interval;
        let confirmed = self
            .overshoots
            .update(envelope, self.ceiling_anchor, self.now);

        // Update ceiling.
        match p.ceiling_mode {
            TrackingMode::Average => {
                // Symmetric EMA — same speed up and down.
                self.ceiling =
                    p.ceiling_fall_coeff * self.ceiling + (1.0 - p.ceiling_fall_coeff) * envelope;
            }
            TrackingMode::Limit => {
                if envelope > self.ceiling {
                    self.last_reached = self.now;
                    if confirmed {
                        self.ceiling = envelope;
                        self.ceiling_anchor = envelope;
                    } else {
                        self.ceiling = envelope.min(self.ceiling_anchor * Self::CEILING_MAX_RISE);
                    }
                } else {
                    if envelope >= Self::CEILING_REACH * self.ceiling {
                        self.last_reached = self.now;
                    }
                    let coeff = if self.now - self.last_reached > Self::CEILING_UNREACHED_SECS {
                        p.ceiling_fast_fall_coeff
                    } else {
                        p.ceiling_fall_coeff
                    };
                    self.ceiling = coeff * self.ceiling + (1.0 - coeff) * envelope;
                }
                self.ceiling_anchor = self.ceiling_anchor.min(self.ceiling);
                self.ceiling_anchor = p.anchor_rise_coeff * self.ceiling_anchor
                    + (1.0 - p.anchor_rise_coeff) * self.ceiling;
            }
        }

        // Update floor. It tracks the envelope clamped to the ceiling, so an
        // outlier the ceiling has refused cannot drag the floor up either.
        let bounded = envelope.min(self.ceiling);
        match p.floor_mode {
            TrackingMode::Average => {
                let coeff = if bounded > self.floor {
                    p.floor_rise_coeff
                } else {
                    p.floor_fall_coeff
                };
                self.floor = coeff * self.floor + (1.0 - coeff) * bounded;
            }
            TrackingMode::Limit => {
                if bounded < self.floor {
                    self.floor = bounded; // instant drop to minimum
                } else {
                    self.floor = p.floor_limit_rise_coeff * self.floor
                        + (1.0 - p.floor_limit_rise_coeff) * bounded;
                }
            }
        }

        if envelope < self.gate {
            return 0.0;
        }
        let range = (self.ceiling - self.floor).max(Self::REL_MIN_RANGE * self.ceiling);
        ((envelope - self.floor) / range).clamp(0.0, 1.0)
    }
}

/// Symmetric one-pole IIR smoother: `y[n] = coeff * y[n-1] + (1 - coeff) * x[n]`.
/// Coefficient is supplied per update since it is typically shared across
/// many smoother instances.
#[derive(Default)]
struct OnePoleSmoother {
    state: f32,
}

impl OnePoleSmoother {
    fn update(&mut self, coeff: f32, input: f32) -> f32 {
        self.state = coeff * self.state + (1.0 - coeff) * input;
        self.state
    }
}

/// A one-pole smoother coefficient derived from a time constant and the audio
/// buffer update rate. Recomputes the expensive `exp()` only when either
/// changes.
#[derive(Default)]
struct SmootherCoeff {
    time_secs: f32,
    update_rate: f32,
    coeff: f32,
}

impl SmootherCoeff {
    /// Refresh the cached coefficient from the current time constant and
    /// update rate. Safe to call every tick.
    fn refresh(&mut self, time_secs: f32, update_rate: f32) {
        if time_secs == self.time_secs && update_rate == self.update_rate {
            return;
        }
        self.time_secs = time_secs;
        self.update_rate = update_rate;
        self.coeff = if time_secs <= 0.0 || update_rate <= 0.0 {
            0.0
        } else {
            (-1.0 / (time_secs * update_rate)).exp()
        };
    }

    fn get(&self) -> f32 {
        self.coeff
    }
}

/// Envelope chain for one output band:
/// Hilbert |z(t)| → fast envelope → slow envelope → smoother → normalizer.
struct BandChain {
    hilbert: HilbertTransform,
    fast_envelope: EnvelopeFollowerProcessor,
    slow_envelope: EnvelopeFollowerProcessor,
    smoother: OnePoleSmoother,
    normalizer: AdaptiveNormalizer,
}

impl BandChain {
    /// `pre_gain` is the fixed gain applied to this band's signal ahead of
    /// the chain, so the normalizer can gate at input level.
    fn new(
        context: &mut AudioContext,
        slow_attack: Duration,
        slow_release: Duration,
        pre_gain: f32,
    ) -> Self {
        Self {
            hilbert: HilbertTransform::new(),
            fast_envelope: make_envelope(context, FAST_ATTACK, FAST_RELEASE),
            slow_envelope: make_envelope(context, slow_attack, slow_release),
            smoother: OnePoleSmoother::default(),
            normalizer: AdaptiveNormalizer::new(pre_gain),
        }
    }

    /// Run one band-limited sample through the envelope followers.
    fn process_sample(&mut self, sample: f32, ctx: &mut AudioContext) {
        let amplitude = self.hilbert.envelope(f64::from(sample)) as f32;
        self.fast_envelope.m_process(ctx, amplitude);
        let fast_val = self.fast_envelope.handle().state();
        self.slow_envelope.m_process(ctx, fast_val);
    }

    /// Finish a buffer: smooth the slow envelope and normalize it, returning
    /// the band's output.
    fn finish(&mut self, smooth_coeff: f32, norm: &NormalizerParams) -> f32 {
        let smoothed = self
            .smoother
            .update(smooth_coeff, self.slow_envelope.handle().state());
        self.normalizer.process(smoothed, norm)
    }

    fn set_slow_envelope(&mut self, attack: Duration, release: Duration) {
        self.slow_envelope.handle().set_attack(attack);
        self.slow_envelope.handle().set_release(release);
    }

    fn stages(&self) -> BandStages {
        BandStages {
            smoothed: self.smoother.state,
            floor: self.normalizer.floor,
            ceiling: self.normalizer.ceiling,
        }
    }
}

/// One output band's intermediate values as of the most recently processed
/// buffer: the smoothed envelope entering the normalizer, and the floor and
/// ceiling the normalizer is currently tracking.
#[derive(Debug, Clone, Copy)]
pub struct BandStages {
    pub smoothed: f32,
    pub floor: f32,
    pub ceiling: f32,
}

pub struct Processor {
    settings: ProcessorSettings,
    filter_cutoff: f32,
    envelope_attack: f32,
    envelope_release: f32,
    channel_count: usize,
    context: AudioContext,

    /// Envelope ring buffer producers — one per output band.
    envelope_producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],

    /// Lowpass filter feeding band 0.
    lowpass_filter: FilterProcessor<f32>,
    /// Wavelet decomposition feeding bands 1..NUM_OUTPUT_BANDS.
    wavelet: WaveletDecomposition,
    /// One envelope chain per output band, in output order: 0 is the
    /// lowpass band, 1..NUM_OUTPUT_BANDS the wavelet bands in ascending
    /// frequency order.
    bands: [BandChain; NUM_OUTPUT_BANDS],
    /// Cached smoother coefficient shared across every band's smoother.
    smooth_coeff: SmootherCoeff,
    /// Normalizer coefficients shared across every band's normalizer.
    norm_params: NormalizerParams,

    /// Automatic input gain trim.
    auto_trim: AutoTrim,
}

/// Fixed gain applied to an output band to flatten the 1/f power spectrum of
/// music: `2^band`, so higher-frequency bands get more boost and the lowpass
/// band none.
fn whitening_gain(output_band: usize) -> f32 {
    (1 << output_band) as f32
}

/// The output band that wavelet level `level` feeds: levels count down from
/// the highest octave, output bands count up from the lowpass band.
fn output_band_of_level(level: usize) -> usize {
    NUM_LEVELS - level
}

fn make_envelope(
    context: &mut AudioContext,
    attack: Duration,
    release: Duration,
) -> EnvelopeFollowerProcessor {
    let mut env = EnvelopeFollowerProcessor::new(attack, release);
    env.m_prepare(context);
    env
}

impl Processor {
    pub fn new(
        handle: ProcessorSettings,
        sample_rate: u32,
        channel_count: usize,
        envelope_producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],
    ) -> Self {
        let mut context: AudioContext = AudioProcessorSettings {
            sample_rate: sample_rate as f32,
            input_channels: channel_count,
            output_channels: channel_count,
            ..Default::default()
        }
        .into();
        let n = context.settings.input_channels;

        let filter_cutoff = handle.filter_cutoff.get();
        let envelope_attack = handle.envelope_attack.get();
        let envelope_release = handle.envelope_release.get();
        let slow_attack = Duration::from_secs_f32(envelope_attack);
        let slow_release = Duration::from_secs_f32(envelope_release);

        let mut lowpass_filter = FilterProcessor::new(FilterType::LowPass);
        lowpass_filter.set_cutoff(filter_cutoff);
        lowpass_filter.m_prepare(&mut context);

        let bands = std::array::from_fn(|band| {
            BandChain::new(
                &mut context,
                slow_attack,
                slow_release,
                whitening_gain(band),
            )
        });

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: n,
            context,
            envelope_producers,
            lowpass_filter,
            wavelet: WaveletDecomposition::new(),
            bands,
            smooth_coeff: SmootherCoeff::default(),
            norm_params: NormalizerParams::new(),
            auto_trim: AutoTrim::new(),
        }
    }

    /// Read an output band's intermediate stage values. Index 0 is the
    /// lowpass band; 1..NUM_OUTPUT_BANDS are the wavelet bands in ascending
    /// frequency order.
    pub fn band_stages(&self, output_band: usize) -> Option<BandStages> {
        self.bands.get(output_band).map(BandChain::stages)
    }

    fn maybe_update_parameters(&mut self, update_rate: f32) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            debug!("Updating filter cutoff to {new_filter_cutoff}");
            self.filter_cutoff = new_filter_cutoff;
            self.lowpass_filter.set_cutoff(new_filter_cutoff);
        }

        let new_attack = self.settings.envelope_attack.get();
        let new_release = self.settings.envelope_release.get();
        if new_attack != self.envelope_attack || new_release != self.envelope_release {
            debug!("Updating envelope parameters to {new_attack}, {new_release}");
            self.envelope_attack = new_attack;
            self.envelope_release = new_release;
            let attack = Duration::from_secs_f32(new_attack);
            let release = Duration::from_secs_f32(new_release);
            for band in &mut self.bands {
                band.set_slow_envelope(attack, release);
            }
        }

        self.smooth_coeff
            .refresh(self.settings.output_smoothing.get(), update_rate);
        self.norm_params.refresh(&self.settings, update_rate);
    }

    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        if interleaved_buffer.is_empty() {
            return;
        }

        let frames = interleaved_buffer.len() / self.channel_count.max(1);
        let update_rate = if frames > 0 {
            self.context.settings.sample_rate / frames as f32
        } else {
            1000.0
        };

        self.maybe_update_parameters(update_rate);

        let mut raw_peak: f32 = 0.0;
        let auto_trim_enabled = self.settings.auto_trim_enabled.load(Ordering::Relaxed);

        // Either manual gain or auto-trim, never both.
        let effective_gain = if auto_trim_enabled {
            self.auto_trim.gain
        } else {
            self.settings.gain.get()
        };

        let ch_count_f = self.channel_count as f32;

        for frame in interleaved_buffer.chunks(self.channel_count) {
            // Both paths run on the mono mix; the trim watches the hottest
            // channel, since headroom is per channel.
            let mut sum = 0.0_f32;
            for raw_sample in frame {
                raw_peak = raw_peak.max(raw_sample.abs());
                sum += raw_sample;
            }
            let mono = sum / ch_count_f * effective_gain;

            let filtered = self.lowpass_filter.m_process(&mut self.context, mono);
            self.bands[0].process_sample(filtered, &mut self.context);

            let bands = &mut self.bands;
            let ctx = &mut self.context;
            self.wavelet.push(mono, |level, sample| {
                // The residual is the lowpass band's job.
                if level == NUM_LEVELS {
                    return;
                }
                let band = output_band_of_level(level);
                bands[band].process_sample(sample * whitening_gain(band), ctx);
            });
        }

        // Update auto-trim from the pre-gain peak, so that the trim never
        // feeds back on its own output.
        if auto_trim_enabled {
            self.auto_trim.set_params(update_rate);
            self.auto_trim.update(raw_peak);
            self.settings.auto_trim_gain.set(self.auto_trim.gain);
        }

        let coeff = self.smooth_coeff.get();
        let mut output_bands = [0.0_f32; NUM_OUTPUT_BANDS];
        for (out, band) in output_bands.iter_mut().zip(&mut self.bands) {
            *out = band.finish(coeff, &self.norm_params);
        }

        // Push normalized envelopes to ring buffers for the GUI viewer.
        for (producer, &val) in self.envelope_producers.iter_mut().zip(&output_bands) {
            producer.push(val);
        }

        // Write the active band's value to the shared envelope atomic.
        let active = self.settings.active_band.load(Ordering::Relaxed) as usize;
        let active = if active >= NUM_OUTPUT_BANDS {
            0
        } else {
            active
        };
        self.settings.envelope.set(output_bands[active]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_producers() -> [EnvelopeProducer; NUM_OUTPUT_BANDS] {
        envelope_ring_buffers().producers
    }

    /// An auto-trim ticking at 1 kHz, so iteration counts read as ms.
    fn trim_at_1khz() -> AutoTrim {
        let mut trim = AutoTrim::new();
        trim.set_params(1000.0);
        trim
    }

    #[test]
    fn auto_trim_boosts_quiet_signal() {
        let mut trim = trim_at_1khz();
        assert!((trim.gain - 1.0).abs() < 1e-6);

        // A consistently quiet signal (0.2 peak) wants +14 dB, clamped to
        // +10 dB. With a 5 s gain half-life, 30 s gets within 2% of it.
        for _ in 0..30000 {
            trim.update(0.2);
        }
        assert!(
            trim.gain > 2.8,
            "Trim should boost quiet signal, got {:.3}",
            trim.gain
        );
        assert!(
            trim.gain <= AutoTrim::db_to_linear(AutoTrim::MAX_GAIN_DB) + 0.01,
            "Trim should not exceed MAX_GAIN, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn auto_trim_reduces_loud_signal() {
        let mut trim = trim_at_1khz();

        // A consistently loud signal (1.5 peak) wants -3.5 dB. With a 5 s
        // gain half-life, 15 s gets within 1 dB of it.
        for _ in 0..15000 {
            trim.update(1.5);
        }
        assert!(
            trim.gain < 0.75,
            "Trim should reduce loud signal, got {:.3}",
            trim.gain
        );
        assert!(
            trim.gain >= AutoTrim::db_to_linear(AutoTrim::MIN_GAIN_DB) - 0.01,
            "Trim should not go below MIN_GAIN, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn auto_trim_stays_near_unity_at_target() {
        let mut trim = trim_at_1khz();

        // Feed signal right at target level.
        for _ in 0..5000 {
            trim.update(AutoTrim::TARGET);
        }
        assert!(
            (trim.gain - 1.0).abs() < 0.05,
            "Trim should stay near 1.0 for target-level signal, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn auto_trim_ignores_silence() {
        let mut trim = trim_at_1khz();

        // Silence and idle noise sit below the silence threshold, so the
        // trim never boosts toward the target — including after music, when
        // the tracked peak is still decaying.
        for _ in 0..500 {
            trim.update(1.0);
        }
        for _ in 0..5000 {
            trim.update(0.0);
        }
        for _ in 0..5000 {
            trim.update(0.005);
        }
        assert!(
            (trim.gain - 1.0).abs() < 0.01,
            "Trim should stay at 1.0 during silence, got {:.3}",
            trim.gain
        );
    }

    #[test]
    fn normalizer_clock_keeps_time_after_hours() {
        let mut params = NormalizerParams::new();
        params.refresh(&ProcessorSettingsInner::default(), 750.0);
        let mut norm = AdaptiveNormalizer::new(1.0);
        // Nine hours into a show.
        norm.now = 9.0 * 3600.0;
        let start = norm.now;
        for _ in 0..750 {
            norm.process(0.5, &params);
        }
        let elapsed = norm.now - start;
        assert!(
            (elapsed - 1.0).abs() < 1e-3,
            "750 updates at 750 Hz should advance the clock by 1 s, got {elapsed}"
        );
    }

    #[test]
    fn auto_trim_disabled_stays_at_unity() {
        let settings = ProcessorSettings::default();
        settings.auto_trim_enabled.store(false, Ordering::Relaxed); // disabled
        let mut processor = Processor::new(settings.clone(), 48000, 1, test_producers());

        // Feed quiet signal — without trim, gain should stay at 1.0.
        let buffer: Vec<f32> = vec![0.1; 48];
        for _ in 0..100 {
            processor.process(&buffer);
        }
        assert!(
            (settings.auto_trim_gain.get() - 1.0).abs() < 0.01,
            "Auto-trim gain should stay at 1.0 when disabled, got {:.3}",
            settings.auto_trim_gain.get()
        );
    }

    #[test]
    fn processor_produces_envelope_from_sine() {
        let settings = ProcessorSettings::default();
        settings.auto_trim_enabled.store(false, Ordering::Relaxed); // disable trim for deterministic test
        let mut processor = Processor::new(settings.clone(), 48000, 1, test_producers());

        // Feed a 100Hz sine for 1 second.
        let sample_rate = 48000.0_f32;
        let total_samples = 48000;
        let buffer_size = 48;
        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    (2.0 * std::f32::consts::PI * 100.0 * t).sin() * 0.7
                })
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        let envelope = settings.envelope.get();
        assert!(
            envelope > 0.3,
            "Envelope should be non-trivial after 1s of 100Hz sine, got {:.3}",
            envelope
        );
    }

    /// Helper: generate a mono sine buffer and feed it through a processor
    /// for the given duration. Returns the final envelope value.
    fn run_processor_with_sine(
        amplitude: f32,
        freq_hz: f32,
        duration_secs: f32,
        settings: &ProcessorSettings,
    ) -> f32 {
        let sample_rate = 48000_u32;
        let buffer_size = 48;
        let total_samples = (duration_secs * sample_rate as f32) as usize;
        let mut processor = Processor::new(settings.clone(), sample_rate, 1, test_producers());

        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| {
                    let t = i as f32 / sample_rate as f32;
                    (2.0 * std::f32::consts::PI * freq_hz * t).sin() * amplitude
                })
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        settings.envelope.get()
    }

    #[test]
    fn auto_trim_converges_quiet_signal_through_processor() {
        // Feed a quiet 100Hz sine (amplitude 0.1) through the full processor
        // with auto-trim enabled. Desired gain = 1.0/0.1 = +20 dB, clamped
        // to +10 dB (3.162x). With a 5 s half-life, 30 s gets within 2%.
        let settings = ProcessorSettings::default();

        let _envelope = run_processor_with_sine(0.1, 100.0, 30.0, &settings);

        let trim_gain = settings.auto_trim_gain.get();
        assert!(
            trim_gain > 1.5,
            "Auto-trim gain should be well above unity for quiet signal, got {:.3}",
            trim_gain
        );
    }

    #[test]
    fn auto_trim_converges_loud_signal_through_processor() {
        // Feed a loud 100Hz sine (amplitude 1.5, over unity) through the
        // full processor. The trim should reduce gain so post-gain peaks
        // approach the target (1.0).
        let settings = ProcessorSettings::default();

        let _envelope = run_processor_with_sine(1.5, 100.0, 10.0, &settings);

        let trim_gain = settings.auto_trim_gain.get();
        assert!(
            trim_gain < 0.8,
            "Auto-trim gain should be well below unity for 1.5x signal, got {:.3}",
            trim_gain
        );
    }

    #[test]
    fn auto_trim_no_feedback_loop() {
        // This is the specific regression test for the feedback loop bug.
        // If the trim feeds back on its own output (post-gain peaks), it
        // will oscillate or converge to the wrong value. If it correctly
        // reads pre-gain peaks, the gain should converge to TARGET/amplitude.
        let settings = ProcessorSettings::default();
        let amplitude = 0.4_f32;

        let _envelope = run_processor_with_sine(amplitude, 100.0, 30.0, &settings);

        let trim_gain = settings.auto_trim_gain.get();
        // Expected: gain ≈ TARGET / amplitude = 1.0 / 0.4 = 2.5
        let expected = AutoTrim::TARGET / amplitude;

        // With a 5 s half-life and 30 s of signal the gain is within 2% of
        // the target. Allow generous tolerance, but catch the feedback loop
        // bug (which converges near 1.0).
        let min_expected = 1.0 + (expected - 1.0) * 0.5; // at least halfway there
        assert!(
            trim_gain > min_expected,
            "Auto-trim gain {:.3} should be converging toward TARGET/amplitude = {:.3} \
             (expected at least {:.3} after 30s). \
             If gain is near 1.0, the trim is feeding back on its own output.",
            trim_gain,
            expected,
            min_expected,
        );
    }
}
