//! A multi-channel audio processor that derives per-band envelopes from its input.
//!
//! Processing chains:
//!   Lowpass: per-channel lowpass → Hilbert |z(t)| → fast envelope → slow envelope
//!   Wavelet: mono D4 decomposition → per-band Hilbert → fast → slow envelope
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
use crate::ring_buffer::EnvelopeProducer;
use crate::wavelet::{NUM_BANDS, NUM_LEVELS, WaveletDecomposition, WaveletType};

/// Fast envelope follower: catches every peak within a cycle.
const FAST_ATTACK: Duration = Duration::from_millis(1);
const FAST_RELEASE: Duration = Duration::new(0, 4_000_000); // 4ms

/// Number of output bands: 1 lowpass sub-bass + 7 wavelet bands.
pub const NUM_OUTPUT_BANDS: usize = 8;

/// Ring buffer capacity: ~16 seconds of history at ~1kHz buffer rate.
pub const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// Band labels in frequency-ascending output order (index 0 = lowpass sub-bass).
pub use crate::wavelet::BAND_LABELS as OUTPUT_BAND_LABELS;

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

    /// Update the trim based on the peak level observed in this buffer.
    /// Returns the current trim gain to apply.
    fn update(&mut self, buffer_peak: f32) -> f32 {
        if buffer_peak < Self::SILENCE {
            return self.gain;
        }

        if buffer_peak > self.peak_tracker {
            self.peak_tracker = buffer_peak;
        } else {
            self.peak_tracker = self.peak_fall_coeff * self.peak_tracker
                + (1.0 - self.peak_fall_coeff) * buffer_peak;
        }

        {
            let desired_db = Self::linear_to_db(Self::TARGET / self.peak_tracker)
                .clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
            self.gain_db = self.gain_coeff * self.gain_db + (1.0 - self.gain_coeff) * desired_db;
            self.gain = Self::db_to_linear(self.gain_db);
        }

        self.gain
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

impl From<u32> for TrackingMode {
    fn from(v: u32) -> Self {
        if v == 1 { Self::Limit } else { Self::Average }
    }
}

impl From<TrackingMode> for u32 {
    fn from(m: TrackingMode) -> Self {
        m as u32
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
    /// EMA coefficient for the anchor's rise toward the ceiling.
    anchor_rise_coeff: f32,
    /// Seconds per update, for timing overshoots.
    interval: f32,
    /// Elapsed processing time in seconds.
    now: f32,
    /// Start times of the most recent overshoots (excursions more than
    /// `OVERSHOOT_MARGIN` above the anchor), oldest first.
    overshoots: [f32; Self::CONFIRM_COUNT],
    /// Whether the envelope is currently in an overshoot.
    in_overshoot: bool,
    /// Peak envelope of the current overshoot; the overshoot ends once the
    /// envelope falls below `OVERSHOOT_END` times this.
    overshoot_peak: f32,
    /// Envelope on the previous update, so an overshoot only begins on a
    /// rising envelope: the decaying tail of a spike never re-triggers.
    prev_envelope: f32,
    /// Envelope level below which the band outputs zero: `NOISE_GATE` scaled
    /// by any fixed gain applied ahead of this band's envelope.
    gate: f32,
    /// EMA coefficients for average-mode floor tracking.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// EMA coefficient for limit-mode floor (slow rise from minimum).
    floor_limit_rise_coeff: f32,
    /// EMA coefficient for ceiling decay.
    ceiling_fall_coeff: f32,
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
    /// An overshoot is an excursion this far above the anchor. Ordinary
    /// beat-to-beat variation in peak height stays well inside this, so only
    /// a real jump in level counts.
    const OVERSHOOT_MARGIN: f32 = 1.5;
    /// An overshoot ends when the envelope falls to this fraction of the
    /// overshoot's peak, so consecutive beats count separately even while
    /// the ceiling is still far below them.
    const OVERSHOOT_END: f32 = 0.5;
    /// This many distinct overshoots within `CONFIRM_WINDOW` seconds — or
    /// one overshoot lasting that long — confirm a real change of level: the
    /// ceiling then snaps to the envelope instead of being nudged. One loud
    /// hit never confirms; a louder passage or a cold start confirms itself
    /// in a few beats, a sustained tone by outlasting the window.
    const CONFIRM_COUNT: usize = 3;
    const CONFIRM_WINDOW: f32 = 1.5;

    /// `pre_gain` is the fixed gain applied to this band's signal ahead of
    /// envelope extraction, so the gate applies at input level.
    fn new(pre_gain: f32) -> Self {
        Self {
            floor: 0.0,
            ceiling: 0.001,
            ceiling_anchor: 0.001,
            anchor_rise_coeff: 0.99,
            interval: 0.001,
            now: 0.0,
            overshoots: [f32::NEG_INFINITY; Self::CONFIRM_COUNT],
            in_overshoot: false,
            overshoot_peak: 0.0,
            prev_envelope: 0.0,
            gate: Self::NOISE_GATE * pre_gain,
            floor_rise_coeff: 0.999,
            floor_fall_coeff: 0.99,
            floor_limit_rise_coeff: 0.999,
            ceiling_fall_coeff: 0.999,
        }
    }

    fn set_params(&mut self, floor_halflife: f32, ceiling_halflife: f32, update_rate: f32) {
        if update_rate <= 0.0 {
            return;
        }
        // Average mode: slow rise, faster fall.
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff = halflife_to_coeff(floor_halflife * 0.2, update_rate);
        // Limit mode: instant drop to min, slow rise back.
        self.floor_limit_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        // Ceiling: instant (bounded) attack, decays at ceiling halflife.
        self.ceiling_fall_coeff = halflife_to_coeff(ceiling_halflife, update_rate);
        self.anchor_rise_coeff = halflife_to_coeff(Self::CEILING_ANCHOR_HALFLIFE, update_rate);
        self.interval = 1.0 / update_rate;
    }

    #[inline]
    fn process(
        &mut self,
        envelope: f32,
        floor_mode: TrackingMode,
        ceiling_mode: TrackingMode,
    ) -> f32 {
        self.now += self.interval;

        // Track overshoots: distinct excursions well above the ceiling's
        // anchor. Several in quick succession confirm a change of level.
        if self.in_overshoot {
            self.overshoot_peak = self.overshoot_peak.max(envelope);
            if envelope < Self::OVERSHOOT_END * self.overshoot_peak {
                self.in_overshoot = false;
            }
        } else if envelope > self.ceiling_anchor * Self::OVERSHOOT_MARGIN
            && envelope > self.prev_envelope
        {
            self.in_overshoot = true;
            self.overshoot_peak = envelope;
            self.overshoots.rotate_left(1);
            self.overshoots[Self::CONFIRM_COUNT - 1] = self.now;
        }
        self.prev_envelope = envelope;
        let latest = self.overshoots[Self::CONFIRM_COUNT - 1];
        let confirmed = self.now - self.overshoots[0] <= Self::CONFIRM_WINDOW
            || (self.in_overshoot && self.now - latest > Self::CONFIRM_WINDOW);

        // Update ceiling.
        match ceiling_mode {
            TrackingMode::Average => {
                // Symmetric EMA — same speed up and down.
                self.ceiling = self.ceiling_fall_coeff * self.ceiling
                    + (1.0 - self.ceiling_fall_coeff) * envelope;
            }
            TrackingMode::Limit => {
                if envelope > self.ceiling {
                    if confirmed {
                        self.ceiling = envelope;
                        self.ceiling_anchor = envelope;
                    } else {
                        self.ceiling = envelope.min(self.ceiling_anchor * Self::CEILING_MAX_RISE);
                    }
                } else {
                    self.ceiling = self.ceiling_fall_coeff * self.ceiling
                        + (1.0 - self.ceiling_fall_coeff) * envelope;
                }
                self.ceiling_anchor = self.ceiling_anchor.min(self.ceiling);
                self.ceiling_anchor = self.anchor_rise_coeff * self.ceiling_anchor
                    + (1.0 - self.anchor_rise_coeff) * self.ceiling;
            }
        }

        // Update floor. It tracks the envelope clamped to the ceiling, so an
        // outlier the ceiling has refused cannot drag the floor up either.
        let bounded = envelope.min(self.ceiling);
        match floor_mode {
            TrackingMode::Average => {
                let coeff = if bounded > self.floor {
                    self.floor_rise_coeff
                } else {
                    self.floor_fall_coeff
                };
                self.floor = coeff * self.floor + (1.0 - coeff) * bounded;
            }
            TrackingMode::Limit => {
                if bounded < self.floor {
                    self.floor = bounded; // instant drop to minimum
                } else {
                    self.floor = self.floor_limit_rise_coeff * self.floor
                        + (1.0 - self.floor_limit_rise_coeff) * bounded;
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
/// buffer update rate. Recomputes the expensive `exp()` only when the time
/// constant changes.
#[derive(Default)]
struct SmootherCoeff {
    time_secs: f32,
    coeff: f32,
}

impl SmootherCoeff {
    /// Refresh the cached coefficient from the current time constant and
    /// update rate. Safe to call every tick.
    fn refresh(&mut self, time_secs: f32, update_rate: f32) {
        if time_secs == self.time_secs {
            return;
        }
        self.time_secs = time_secs;
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

/// Per-audio-channel processing chain for the lowpass path.
struct LowpassChannel {
    filter: FilterProcessor<f32>,
    hilbert: HilbertTransform,
    fast_envelope: EnvelopeFollowerProcessor,
    slow_envelope: EnvelopeFollowerProcessor,
}

impl LowpassChannel {
    /// Run one input sample through the full chain:
    /// lowpass → Hilbert |z(t)| → fast envelope → slow envelope.
    fn process_sample(&mut self, sample: f32, ctx: &mut AudioContext) {
        let filtered = self.filter.m_process(ctx, sample);
        let amplitude = self.hilbert.envelope(filtered as f64) as f32;
        self.fast_envelope.m_process(ctx, amplitude);
        let fast_val = self.fast_envelope.handle().state();
        self.slow_envelope.m_process(ctx, fast_val);
    }

    fn slow_envelope_state(&self) -> f32 {
        self.slow_envelope.handle().state()
    }
}

/// Per-frequency-band processing chain for the wavelet path. Operates on
/// the mono mix at a band-specific (decimated) sample rate.
struct WaveletBand {
    hilbert: HilbertTransform,
    fast_envelope: EnvelopeFollowerProcessor,
    slow_envelope: EnvelopeFollowerProcessor,
    smoother: OnePoleSmoother,
    normalizer: AdaptiveNormalizer,
    context: AudioContext,
}

impl WaveletBand {
    /// Run one decimated sample through the full chain:
    /// whitening → Hilbert → fast envelope → slow envelope.
    fn process_sample(&mut self, sample: f32, whiten: f32) {
        let amp = self.hilbert.envelope((sample * whiten) as f64) as f32;
        self.fast_envelope.m_process(&mut self.context, amp);
        let fast_val = self.fast_envelope.handle().state();
        self.slow_envelope.m_process(&mut self.context, fast_val);
    }

    /// Read the slow envelope and push it through the output smoother.
    fn smoothed_envelope(&mut self, coeff: f32) -> f32 {
        let env_val = self.slow_envelope.handle().state();
        self.smoother.update(coeff, env_val)
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

    /// Lowpass chain: one per audio channel.
    lowpass: Vec<LowpassChannel>,

    /// Output smoother for the lowpass path (applied after per-channel averaging).
    lowpass_smoother: OnePoleSmoother,
    /// Adaptive normalizer for the lowpass envelope.
    lowpass_normalizer: AdaptiveNormalizer,
    /// Cached smoother coefficient shared across the lowpass smoother and
    /// every wavelet-band smoother.
    smooth_coeff: SmootherCoeff,

    /// Automatic input gain trim.
    auto_trim: AutoTrim,

    /// Wavelet decomposition (D4) + per-band envelope extraction.
    wavelet: WaveletDecomposition,
    /// One chain per frequency band. Indexed by wavelet band, not audio channel.
    wavelet_bands: [WaveletBand; NUM_BANDS],
}

/// Fixed gain applied to a wavelet band to flatten the 1/f power spectrum of
/// music: `2^(NUM_LEVELS - band)`, so higher-frequency bands get more boost.
fn whitening_gain(band: usize) -> f32 {
    (1 << (NUM_LEVELS - band)) as f32
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

        let lowpass = (0..n)
            .map(|_| {
                let mut filter = FilterProcessor::new(FilterType::LowPass);
                filter.set_cutoff(filter_cutoff);
                filter.m_prepare(&mut context);
                LowpassChannel {
                    filter,
                    hilbert: HilbertTransform::new(),
                    fast_envelope: make_envelope(&mut context, FAST_ATTACK, FAST_RELEASE),
                    slow_envelope: make_envelope(&mut context, slow_attack, slow_release),
                }
            })
            .collect();

        // Per-band envelope chains for the wavelet decomposition. Each band
        // runs at a decimated sample rate (base / 2^(level+1)).
        let base_sr = sample_rate as f32;
        let wavelet_bands = std::array::from_fn(|band| {
            let level = if band < NUM_LEVELS {
                band
            } else {
                NUM_LEVELS - 1
            };
            let band_sr = base_sr / (1 << (level + 1)) as f32;
            let mut band_ctx: AudioContext = AudioProcessorSettings {
                sample_rate: band_sr,
                input_channels: 1,
                output_channels: 1,
                ..Default::default()
            }
            .into();
            WaveletBand {
                hilbert: HilbertTransform::new(),
                fast_envelope: make_envelope(&mut band_ctx, FAST_ATTACK, FAST_RELEASE),
                slow_envelope: make_envelope(&mut band_ctx, slow_attack, slow_release),
                smoother: OnePoleSmoother::default(),
                normalizer: AdaptiveNormalizer::new(whitening_gain(band)),
                context: band_ctx,
            }
        });

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: n,
            context,
            envelope_producers,
            lowpass,
            lowpass_smoother: OnePoleSmoother::default(),
            lowpass_normalizer: AdaptiveNormalizer::new(1.0),
            smooth_coeff: SmootherCoeff::default(),
            auto_trim: AutoTrim::new(),
            wavelet: WaveletDecomposition::new(WaveletType::Daubechies4),
            wavelet_bands,
        }
    }

    /// Read an output band's intermediate stage values. Index 0 is the
    /// lowpass band; 1..NUM_OUTPUT_BANDS are the wavelet bands in ascending
    /// frequency order. Out-of-range indices read as band 0.
    pub fn band_stages(&self, output_band: usize) -> BandStages {
        if output_band == 0 || output_band >= NUM_OUTPUT_BANDS {
            return BandStages {
                smoothed: self.lowpass_smoother.state,
                floor: self.lowpass_normalizer.floor,
                ceiling: self.lowpass_normalizer.ceiling,
            };
        }
        let band = &self.wavelet_bands[NUM_LEVELS - output_band];
        BandStages {
            smoothed: band.smoother.state,
            floor: band.normalizer.floor,
            ceiling: band.normalizer.ceiling,
        }
    }

    fn maybe_update_parameters(&mut self, update_rate: f32) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            debug!("Updating filter cutoff to {new_filter_cutoff}");
            self.filter_cutoff = new_filter_cutoff;
            for chan in &mut self.lowpass {
                chan.filter.set_cutoff(new_filter_cutoff);
            }
        }

        let new_attack = self.settings.envelope_attack.get();
        let new_release = self.settings.envelope_release.get();
        if new_attack != self.envelope_attack || new_release != self.envelope_release {
            debug!("Updating envelope parameters to {new_attack}, {new_release}");
            self.envelope_attack = new_attack;
            self.envelope_release = new_release;
            let attack = Duration::from_secs_f32(new_attack);
            let release = Duration::from_secs_f32(new_release);
            for chan in &mut self.lowpass {
                chan.slow_envelope.handle().set_attack(attack);
                chan.slow_envelope.handle().set_release(release);
            }
            for band in &mut self.wavelet_bands {
                band.slow_envelope.handle().set_attack(attack);
                band.slow_envelope.handle().set_release(release);
            }
        }

        self.smooth_coeff
            .refresh(self.settings.output_smoothing.get(), update_rate);
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
        let mut input_peak: f32 = 0.0;
        let auto_trim_enabled = self.settings.auto_trim_enabled.load(Ordering::Relaxed);

        // Either manual gain or auto-trim, never both.
        let effective_gain = if auto_trim_enabled {
            self.auto_trim.gain
        } else {
            self.settings.gain.get()
        };

        let ch_count_f = self.channel_count as f32;

        for frame in interleaved_buffer.chunks(self.channel_count) {
            // Compute mono mix for wavelet input.
            let mono = frame.iter().sum::<f32>() / ch_count_f;

            for (chan, raw_sample) in self.lowpass.iter_mut().zip(frame) {
                raw_peak = raw_peak.max(raw_sample.abs());
                let sample = *raw_sample * effective_gain;
                input_peak = input_peak.max(sample.abs());
                chan.process_sample(sample, &mut self.context);
            }

            // Wavelet decomposition -> per-band envelope extraction.
            let mono_gained = mono * effective_gain;
            let bands = &mut self.wavelet_bands;
            self.wavelet.push(mono_gained, |band, sample| {
                // Skip the residual band (== lowpass, redundant with our LP chain).
                if band == NUM_LEVELS {
                    return;
                }
                bands[band].process_sample(sample, whitening_gain(band));
            });
        }

        // Update auto-trim based on the pre-gain peak (raw signal level).
        // We feed raw_peak, not input_peak, to avoid a feedback loop where
        // the trim adjusts based on its own output.
        if auto_trim_enabled {
            self.auto_trim.set_params(update_rate);
            self.auto_trim.update(raw_peak);
            self.settings.auto_trim_gain.set(self.auto_trim.gain);
        }

        let ch_count = self.channel_count as f32;
        let envelope = self
            .lowpass
            .iter()
            .map(LowpassChannel::slow_envelope_state)
            .sum::<f32>()
            / ch_count;

        let coeff = self.smooth_coeff.get();
        let smoothed_lowpass = self.lowpass_smoother.update(coeff, envelope);

        // Normalization params (shared between lowpass and wavelet normalizers).
        let floor_hl = self.settings.norm_floor_halflife.get();
        let ceil_hl = self.settings.norm_ceiling_halflife.get();
        let floor_mode = self.settings.norm_floor_mode.load(Ordering::Relaxed);
        let ceil_mode = self.settings.norm_ceiling_mode.load(Ordering::Relaxed);

        self.lowpass_normalizer
            .set_params(floor_hl, ceil_hl, update_rate);
        let lowpass_norm = self
            .lowpass_normalizer
            .process(smoothed_lowpass, floor_mode, ceil_mode);

        // Wavelet band smoothing + normalization.
        // Build the output array: [lowpass_norm, wavelet_band_6_norm, ..., wavelet_band_0_norm]
        let mut output_bands = [0.0_f32; NUM_OUTPUT_BANDS];
        output_bands[0] = lowpass_norm;

        for (i, band) in self.wavelet_bands.iter_mut().enumerate() {
            band.normalizer.set_params(floor_hl, ceil_hl, update_rate);
            let smoothed = band.smoothed_envelope(coeff);
            let normalized = band.normalizer.process(smoothed, floor_mode, ceil_mode);
            // Map wavelet bands 0-6 to output indices 7-1.
            // output_index = NUM_LEVELS - wavelet_band_index (for bands 0..NUM_LEVELS).
            if i < NUM_LEVELS {
                output_bands[NUM_LEVELS - i] = normalized;
            }
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
    use crate::ring_buffer::envelope_ring_buffer;

    fn test_producers() -> [EnvelopeProducer; NUM_OUTPUT_BANDS] {
        let mut producers = Vec::with_capacity(NUM_OUTPUT_BANDS);
        for _ in 0..NUM_OUTPUT_BANDS {
            let (p, _c) = envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY);
            producers.push(p);
        }
        producers.try_into().ok().expect("correct count")
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
