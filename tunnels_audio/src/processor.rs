//! A multi-channel audio processor that derives per-band envelopes from its input.
//!
//! Every input channel first passes through an automatic trim and a DC
//! blocker. Processing chains:
//!   Lowpass: per-channel lowpass → Hilbert |z(t)| → fast envelope → slow envelope
//!   Wavelet: mono D4 decomposition → per-band Hilbert → fast → slow envelope
//!
//! Each band's slow envelope then passes through the output smoother and an
//! adaptive normalizer, once per buffer.
//!
//! Output: 8 normalized bands (1 lowpass + 7 wavelet), selectable via `active_band`.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{AtomicF32, AudioContext, simple_processor::MonoAudioProcessor};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::debug;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tunnels_lib::transient_indicator::TransientIndicator;

use crate::hilbert::HilbertTransform;
use crate::input_meter::InputMeter;
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
    /// Symmetric output smoothing time constant (seconds). 0 = disabled.
    pub output_smoothing: AtomicF32,

    /// Floor tracking half-life in seconds (slow — adapts to ambient level).
    pub norm_floor_halflife: AtomicF32,
    /// Ceiling tracking half-life, in seconds of music at
    /// `REFERENCE_MOTION_RATE`. The ceiling's memory is really a quantity of
    /// envelope motion, so quiet or still material stretches these seconds
    /// and a silent band holds the ceiling indefinitely.
    pub norm_ceiling_halflife: AtomicF32,

    /// Which band feeds `envelope`: 0 = lowpass, 1-7 = wavelet bands.
    pub active_band: AtomicU32,
}

impl ProcessorSettingsInner {
    const DEFAULT_FILTER_CUTOFF: f32 = 187.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;
    /// Floor half-life: slow enough that a bass line is above its own bed,
    /// fast enough that a held tone stops being news within a phrase or two.
    pub const DEFAULT_FLOOR_HALFLIFE: f32 = 10.0;
    /// Ceiling half-life of about a phrase: each band is measured against
    /// the loudest thing in the phrase it is part of.
    pub const DEFAULT_CEILING_HALFLIFE: f32 = 8.0;

    pub fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
        self.active_band.store(0, Ordering::Relaxed);
        self.norm_floor_halflife.set(Self::DEFAULT_FLOOR_HALFLIFE);
        self.norm_ceiling_halflife
            .set(Self::DEFAULT_CEILING_HALFLIFE);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            filter_cutoff: AtomicF32::new(Self::DEFAULT_FILTER_CUTOFF),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            norm_floor_halflife: AtomicF32::new(Self::DEFAULT_FLOOR_HALFLIFE),
            norm_ceiling_halflife: AtomicF32::new(Self::DEFAULT_CEILING_HALFLIFE),
            active_band: AtomicU32::new(0),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// A slow automatic gain that brings the input's peaks toward a fixed target,
/// within limits, so the level the interface is set to does not decide what
/// sits above the noise gate. It holds still on silence.
struct AutoTrim {
    /// Tracked input peak: instant attack, slow decay.
    peak: f32,
    gain_db: f32,
    /// `gain_db` as a multiplier.
    gain: f32,
    update_rate: f32,
    peak_fall_coeff: f32,
    gain_coeff: f32,
}

impl AutoTrim {
    /// Peak level the trim aims for. Unity: downstream is floating point
    /// and a momentary overshoot costs nothing.
    const TARGET: f32 = 1.0;
    /// How far the trim may go. The boost reaches a feed run well below a
    /// mastered level; past that it would be lifting the interface's own
    /// noise toward the gate.
    const MIN_GAIN_DB: f32 = -10.0;
    const MAX_GAIN_DB: f32 = 20.0;
    /// Peak tracker fall half-life.
    const PEAK_FALL_HALFLIFE: Duration = Duration::from_secs(10);
    /// Gain slew half-life, the same in both directions: slow enough that a
    /// stray peak costs only a gentle, brief dip.
    const GAIN_HALFLIFE: Duration = Duration::from_secs(5);
    /// Input peak below which the trim holds still, so silence and idle
    /// noise are never boosted toward the target.
    const SILENCE: f32 = 0.01;

    fn new() -> Self {
        Self {
            peak: 0.0,
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

    fn linear_to_db(linear: f32) -> f32 {
        20.0 * linear.log10()
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

    /// Update the trim from the peak level of one buffer of input.
    fn update(&mut self, buffer_peak: f32) {
        if buffer_peak < Self::SILENCE {
            return;
        }

        self.peak = if buffer_peak > self.peak {
            buffer_peak
        } else {
            self.peak_fall_coeff * self.peak + (1.0 - self.peak_fall_coeff) * buffer_peak
        };

        let desired_db = Self::linear_to_db(Self::TARGET / self.peak)
            .clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB);
        self.gain_db = self.gain_coeff * self.gain_db + (1.0 - self.gain_coeff) * desired_db;
        self.gain = Self::db_to_linear(self.gain_db);
    }
}

/// Level at or beyond which an input sample is at the converter's ceiling.
const CLIP_LEVEL: f32 = 0.999;

/// Consecutive samples on one channel at that level before it counts as
/// clipping rather than a signal that happens to touch full scale.
const CLIP_RUN: u32 = 3;

/// How long the clip indicator stays lit after the last buffer that clipped.
const CLIP_HOLD: Duration = Duration::from_millis(300);

/// Whether any one channel of an interleaved buffer of `channel_count`
/// channels holds a run of [`CLIP_RUN`] consecutive samples at
/// [`CLIP_LEVEL`] or beyond.
fn buffer_clips(interleaved_buffer: &[f32], channel_count: NonZeroUsize) -> bool {
    let channel_count = channel_count.get();
    (0..channel_count).any(|channel| {
        let mut run = 0;
        interleaved_buffer
            .iter()
            .skip(channel)
            .step_by(channel_count)
            .any(|sample| {
                if sample.abs() >= CLIP_LEVEL {
                    run += 1;
                } else {
                    run = 0;
                }
                run >= CLIP_RUN
            })
    })
}

/// Nepers per second a band's log envelope is taken to move on music. The
/// ceiling's memory is a quantity of envelope motion, and this rate converts
/// it to a half-life in seconds.
///
/// Motion accrues once per buffer, so a longer buffer misses ripple finer
/// than its period and forgets a little more slowly than the half-life says.
pub const REFERENCE_MOTION_RATE: f32 = 4.2;

/// One-pole EMA coefficient that halves the distance to the target every
/// `halflife` at `update_rate` updates per second. A zero half-life means no
/// smoothing.
pub(crate) fn halflife_to_coeff(halflife: Duration, update_rate: f32) -> f32 {
    let halflife_secs = halflife.as_secs_f32();
    if halflife_secs <= 0.0 || update_rate <= 0.0 {
        return 0.0;
    }
    (-f32::ln(2.0) / (halflife_secs * update_rate)).exp()
}

/// A count of seconds as a duration; zero for a count that is negative, not
/// a number, or too large to represent.
fn secs_to_duration(secs: f32) -> Duration {
    Duration::try_from_secs_f32(secs).unwrap_or(Duration::ZERO)
}

/// The normalizer's fixed constants: what it treats as silence and how
/// little range it will stretch to full scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizerTuning {
    /// Minimum normalization range as a fraction of the ceiling. Once the
    /// floor has climbed to within this fraction of the ceiling the output
    /// fades instead of being stretched back to full scale, and a band's
    /// reach to full scale never depends on its absolute level.
    pub rel_min_range: f32,
    /// The lowest level the floor may sit at, so a band whose content is
    /// at or below it outputs zero rather than normalizing idle noise up to
    /// full scale.
    pub noise_gate: f32,
}

impl NormalizerTuning {
    pub const DEFAULT: Self = Self {
        rel_min_range: 0.25,
        noise_gate: 0.01,
    };
}

impl Default for NormalizerTuning {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The normalizer coefficients every band shares, derived from the settings
/// and the buffer update rate. The expensive `exp()`s are recomputed only
/// when a half-life or the update rate changes.
pub(crate) struct NormalizerParams {
    tuning: NormalizerTuning,
    /// Nepers the ceiling decays per neper of log-envelope motion.
    ceiling_forget: f32,
    /// The half-life the forgetting rate was derived from.
    ceiling_halflife: f32,
    /// Floor follower coefficients: the floor rises at the floor half-life
    /// and falls at a fifth of it.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// The inputs the coefficients were derived from.
    floor_halflife: Duration,
    update_rate: f32,
}

impl NormalizerParams {
    /// The floor falls this much faster than it rises.
    const FLOOR_FALL_RATIO: f32 = 0.2;

    fn new(tuning: NormalizerTuning) -> Self {
        Self {
            tuning,
            ceiling_forget: 0.0,
            ceiling_halflife: 0.0,
            floor_rise_coeff: 0.0,
            floor_fall_coeff: 0.0,
            floor_halflife: Duration::ZERO,
            update_rate: 0.0,
        }
    }

    /// Refresh from the settings for the given update rate. Safe to call
    /// every buffer.
    fn refresh(&mut self, settings: &ProcessorSettingsInner, update_rate: f32) {
        let ceiling_halflife = settings.norm_ceiling_halflife.get();
        if ceiling_halflife != self.ceiling_halflife {
            self.ceiling_halflife = ceiling_halflife;
            // A non-positive half-life means the ceiling never forgets.
            self.ceiling_forget = if ceiling_halflife > 0.0 {
                std::f32::consts::LN_2 / (REFERENCE_MOTION_RATE * ceiling_halflife)
            } else {
                0.0
            };
        }
        let floor_halflife = secs_to_duration(settings.norm_floor_halflife.get());
        if update_rate <= 0.0
            || (floor_halflife == self.floor_halflife && update_rate == self.update_rate)
        {
            return;
        }
        self.floor_halflife = floor_halflife;
        self.update_rate = update_rate;
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff =
            halflife_to_coeff(floor_halflife.mul_f32(Self::FLOOR_FALL_RATIO), update_rate);
    }
}

/// Adaptive envelope normalizer: tracks a floor and ceiling,
/// outputs `(envelope - floor) / (ceiling - floor)` clamped to [0, 1].
///
/// The ceiling is a peak follower whose decay is clocked by the envelope's
/// own motion rather than by time: it rises to any envelope above it at once
/// and forgets a fixed number of nepers for every neper the log envelope
/// moves, up or down, set by the ceiling half-life. A hit therefore costs the ceiling the same whether
/// the music is fast or slow, a level drop is forgotten within a few hits
/// at any tempo, and a pause or a held tone — no motion — leaves the
/// ceiling where it was.
pub(crate) struct AdaptiveNormalizer {
    /// Follows the envelope slowly upward and faster downward: the bed the
    /// output is measured from.
    floor: AsymmetricOnePole,
    ceiling: f32,
    /// Log envelope on the previous update, clamped at the noise gate.
    prev_log_envelope: f32,
}

impl AdaptiveNormalizer {
    /// The ceiling a normalizer starts from, before it has heard anything:
    /// half a full-scale tone's envelope on the lowpass band. Starting high
    /// under-reports until the real level is learnt, rather than calling the
    /// first sound full scale; louder material costs one hit's worth of
    /// clipping.
    const INITIAL_CEILING: f32 = 0.5;

    pub(crate) fn new(tuning: &NormalizerTuning) -> Self {
        Self {
            floor: AsymmetricOnePole::default(),
            ceiling: Self::INITIAL_CEILING,
            prev_log_envelope: tuning.noise_gate.ln(),
        }
    }

    #[inline]
    pub(crate) fn process(&mut self, envelope: f32, p: &NormalizerParams) -> f32 {
        let t = &p.tuning;
        // Update ceiling: instant attack, decay per unit of envelope motion.
        let log_envelope = envelope.max(t.noise_gate).ln();
        let motion = (log_envelope - self.prev_log_envelope).abs();
        self.prev_log_envelope = log_envelope;
        self.ceiling = envelope.max(self.ceiling * (-p.ceiling_forget * motion).exp());

        // Update floor. It never exceeds the ceiling.
        self.floor.rise = p.floor_rise_coeff;
        self.floor.fall = p.floor_fall_coeff;
        let floor = self.floor.step(envelope.min(self.ceiling));

        // The gate is the lowest level the floor can sit at, so the output
        // reaches zero by arriving there rather than by being cut off.
        let floor = floor.max(t.noise_gate);
        // The range is never narrower than the gate either, so a band only
        // reaches full scale once its ceiling stands clear of the noise it
        // would otherwise be stretching.
        let range = (self.ceiling - floor)
            .max(t.rel_min_range * self.ceiling)
            .max(t.noise_gate);
        ((envelope - floor) / range).clamp(0.0, 1.0)
    }
}

/// One-pole DC blocker: `y[n] = x[n] - x[n-1] + coeff * y[n-1]`, the
/// difference equation of a series capacitor, removing any constant offset.
struct DcBlocker {
    coeff: f32,
    x_prev: f32,
    y_prev: f32,
}

impl DcBlocker {
    /// Corner frequency: within 0.2 dB of flat from a few tens of hertz up.
    const CORNER_HZ: f32 = 5.0;

    fn new(sample_rate: f32) -> Self {
        Self {
            coeff: 1.0 - std::f32::consts::TAU * Self::CORNER_HZ / sample_rate,
            x_prev: 0.0,
            y_prev: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = input - self.x_prev + self.coeff * self.y_prev;
        self.x_prev = input;
        self.y_prev = output;
        output
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

/// One-pole follower with separate coefficients for rising and falling
/// input: `y[n] = c * y[n-1] + (1 - c) * x[n]` with `c` the rise coefficient
/// while `x[n] > y[n-1]` and the fall coefficient otherwise. A coefficient of
/// zero follows the input at once in that direction.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AsymmetricOnePole {
    pub(crate) rise: f32,
    pub(crate) fall: f32,
    state: f32,
}

impl AsymmetricOnePole {
    #[inline]
    pub(crate) fn step(&mut self, input: f32) -> f32 {
        let c = if input > self.state {
            self.rise
        } else {
            self.fall
        };
        self.state = c * self.state + (1.0 - c) * input;
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

pub struct Processor {
    settings: ProcessorSettings,
    filter_cutoff: f32,
    envelope_attack: f32,
    envelope_release: f32,
    /// Interleaved channels per frame.
    channel_count: NonZeroUsize,
    context: AudioContext,

    /// Envelope ring buffer producers — one per output band.
    envelope_producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],

    /// Brings the input to a consistent level ahead of everything else.
    auto_trim: AutoTrim,
    /// Lit by a buffer in which the input clipped, and held for [`CLIP_HOLD`].
    clip_indicator: TransientIndicator,
    /// Where the trim and the clip indicator are published each buffer.
    input_meter: Arc<InputMeter>,
    /// One per channel: removes any offset from the input before it reaches
    /// either chain.
    dc_blockers: Box<[DcBlocker]>,

    /// Lowpass chain: one per audio channel.
    lowpass: Vec<LowpassChannel>,

    /// Output smoother for the lowpass path (applied after per-channel averaging).
    lowpass_smoother: OnePoleSmoother,
    /// Adaptive normalizer for the lowpass envelope.
    lowpass_normalizer: AdaptiveNormalizer,
    /// Cached smoother coefficient shared across the lowpass smoother and
    /// every wavelet-band smoother.
    smooth_coeff: SmootherCoeff,
    /// Normalizer coefficients shared across every band's normalizer.
    norm_params: NormalizerParams,

    /// Wavelet decomposition (D4) + per-band envelope extraction.
    wavelet: WaveletDecomposition,
    /// One chain per frequency band. Indexed by wavelet band, not audio channel.
    wavelet_bands: [WaveletBand; NUM_BANDS],
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
        channel_count: NonZeroUsize,
        envelope_producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],
    ) -> Self {
        let n = channel_count.get();
        let mut context: AudioContext = AudioProcessorSettings {
            sample_rate: sample_rate as f32,
            input_channels: n,
            output_channels: n,
            ..Default::default()
        }
        .into();

        let filter_cutoff = handle.filter_cutoff.get();
        let envelope_attack = handle.envelope_attack.get();
        let envelope_release = handle.envelope_release.get();
        let slow_attack = Duration::from_secs_f32(envelope_attack);
        let slow_release = Duration::from_secs_f32(envelope_release);
        let tuning = NormalizerTuning::DEFAULT;

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
                normalizer: AdaptiveNormalizer::new(&tuning),
                context: band_ctx,
            }
        });

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count,
            context,
            envelope_producers,
            auto_trim: AutoTrim::new(),
            clip_indicator: TransientIndicator::new(CLIP_HOLD),
            input_meter: Arc::new(InputMeter::default()),
            dc_blockers: (0..n).map(|_| DcBlocker::new(base_sr)).collect(),
            lowpass,
            lowpass_smoother: OnePoleSmoother::default(),
            lowpass_normalizer: AdaptiveNormalizer::new(&tuning),
            smooth_coeff: SmootherCoeff::default(),
            norm_params: NormalizerParams::new(tuning),
            wavelet: WaveletDecomposition::new(WaveletType::Daubechies4),
            wavelet_bands,
        }
    }

    /// The meter this processor publishes its trim and clip indicator to.
    ///
    /// The processor holds the meter's only strong reference, so the handle
    /// upgrades exactly as long as the processor exists.
    pub fn input_meter(&self) -> Weak<InputMeter> {
        Arc::downgrade(&self.input_meter)
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
        self.norm_params.refresh(&self.settings, update_rate);
    }

    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        // Exact digital silence decays the filters and followers into
        // subnormal values, which some CPUs process many times more slowly.
        crate::denormals::flush_subnormals_to_zero();
        if interleaved_buffer.is_empty() {
            return;
        }

        let channel_count = self.channel_count.get();
        let frames = interleaved_buffer.len() / channel_count;
        if frames == 0 {
            return;
        }
        let sample_rate = self.context.settings.sample_rate;
        let update_rate = sample_rate / frames as f32;

        self.maybe_update_parameters(update_rate);

        // The trim reads the input before its own gain reaches it, so it
        // cannot chase itself.
        let input_peak = interleaved_buffer
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        self.auto_trim.set_params(update_rate);
        self.auto_trim.update(input_peak);
        let gain = self.auto_trim.gain;

        self.clip_indicator.update_state(
            secs_to_duration(frames as f32 / sample_rate),
            buffer_clips(interleaved_buffer, self.channel_count),
        );
        self.input_meter
            .set(self.auto_trim.gain_db, self.clip_indicator.state());

        let ch_count_f = channel_count as f32;

        for frame in interleaved_buffer.chunks(channel_count) {
            let mut mono = 0.0;
            for ((chan, dc_blocker), raw_sample) in self
                .lowpass
                .iter_mut()
                .zip(self.dc_blockers.iter_mut())
                .zip(frame)
            {
                // A device that hands us a non-finite sample would otherwise
                // poison the filters and followers for the rest of the show:
                // their state is recursive, so a NaN in it never washes out.
                let sample = raw_sample * gain;
                let sample = if sample.is_finite() { sample } else { 0.0 };
                let sample = dc_blocker.process(sample);
                mono += sample;
                chan.process_sample(sample, &mut self.context);
            }
            let mono = mono / ch_count_f;

            // Wavelet decomposition -> per-band envelope extraction.
            // Whitening: multiply by 2^(NUM_LEVELS - level) to correct for
            // the 1/f power spectrum of music. Higher bands get more boost.
            let bands = &mut self.wavelet_bands;
            self.wavelet.push(mono, |band, sample| {
                // Skip the residual band (== lowpass, redundant with our LP chain).
                if band == NUM_LEVELS {
                    return;
                }
                let whiten = (1 << (NUM_LEVELS - band)) as f32;
                bands[band].process_sample(sample, whiten);
            });
        }

        let envelope = self
            .lowpass
            .iter()
            .map(LowpassChannel::slow_envelope_state)
            .sum::<f32>()
            / ch_count_f;

        let coeff = self.smooth_coeff.get();
        let smoothed_lowpass = self.lowpass_smoother.update(coeff, envelope);
        let lowpass_norm = self
            .lowpass_normalizer
            .process(smoothed_lowpass, &self.norm_params);

        // Wavelet band smoothing + normalization.
        // Build the output array: [lowpass_norm, wavelet_band_6_norm, ..., wavelet_band_0_norm]
        let mut output_bands = [0.0_f32; NUM_OUTPUT_BANDS];
        output_bands[0] = lowpass_norm;

        for (i, band) in self.wavelet_bands.iter_mut().enumerate() {
            let smoothed = band.smoothed_envelope(coeff);
            let normalized = band.normalizer.process(smoothed, &self.norm_params);
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

    fn channels(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("a test processor has at least one channel")
    }

    fn test_producers() -> [EnvelopeProducer; NUM_OUTPUT_BANDS] {
        let mut producers = Vec::with_capacity(NUM_OUTPUT_BANDS);
        for _ in 0..NUM_OUTPUT_BANDS {
            let (p, _c) = envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY);
            producers.push(p);
        }
        producers.try_into().ok().expect("correct count")
    }

    #[test]
    fn processor_produces_envelope_from_sine() {
        let settings = ProcessorSettings::default();
        let envelope = run_processor(&settings, 48000, 1.0, |t| {
            0.7 * (std::f32::consts::TAU * 100.0 * t).sin()
        });
        assert!(
            envelope > 0.3,
            "Envelope should be non-trivial after 1s of 100Hz sine, got {:.3}",
            envelope
        );
    }

    /// A band whose input is a constant offset is not hearing anything, so
    /// its envelope stays at zero.
    #[test]
    fn dc_offset_produces_no_envelope() {
        let settings = ProcessorSettings::default();
        let envelope = run_processor(&settings, 48000, 1.0, |_| 0.5);
        assert!(
            envelope < 0.05,
            "a 0.5 DC offset should read as silence, got {envelope:.3}"
        );
    }

    /// A processor that has heard nothing has no idea how loud the music
    /// is, so it starts quiet and works its way up rather than reporting
    /// full scale from the first sample it sees.
    #[test]
    fn a_new_processor_does_not_start_at_full_scale() {
        let settings = ProcessorSettings::default();
        let sample_rate = 48000_u32;
        let mut processor =
            Processor::new(settings.clone(), sample_rate, channels(1), test_producers());
        // A third of full scale, so full scale is unambiguously wrong.
        let mut peak = 0.0_f32;
        for buffer in 0..80 {
            let samples: Vec<f32> = (0..64)
                .map(|i| {
                    let t = (buffer * 64 + i) as f32 / sample_rate as f32;
                    0.3 * (2.0 * std::f32::consts::PI * 60.0 * t).sin()
                })
                .collect();
            processor.process(&samples);
            peak = peak.max(settings.envelope.get());
        }
        assert!(
            peak < 0.9,
            "the first 107 ms of a quiet tone should not read as full scale, got {peak:.3}"
        );
    }

    /// The trim follows the input's level, stops at its limits, and treats
    /// silence as nothing to act on.
    #[test]
    fn auto_trim_follows_level_and_holds_on_silence() {
        fn trim_at_1khz() -> AutoTrim {
            let mut trim = AutoTrim::new();
            trim.set_params(1000.0);
            trim
        }
        fn feed(trim: &mut AutoTrim, peak: f32, updates: usize) {
            for _ in 0..updates {
                trim.update(peak);
            }
        }

        // Right at target: stays put, and silence afterwards leaves it there.
        let mut trim = trim_at_1khz();
        feed(&mut trim, AutoTrim::TARGET, 5000);
        assert!(
            (trim.gain - 1.0).abs() < 0.05,
            "at target the trim should hold near 1.0, got {:.3}",
            trim.gain
        );
        feed(&mut trim, 0.0, 5000);
        feed(&mut trim, 0.005, 5000);
        assert!(
            (trim.gain - 1.0).abs() < 0.01,
            "silence should leave the trim where it was, got {:.3}",
            trim.gain
        );

        // A quiet feed is boosted, up to the limit.
        let mut trim = trim_at_1khz();
        feed(&mut trim, 0.02, 60000);
        let max = AutoTrim::db_to_linear(AutoTrim::MAX_GAIN_DB);
        assert!(
            trim.gain > 0.9 * max && trim.gain <= max + 0.01,
            "a feed 34 dB down should reach the boost limit {max:.1}, got {:.3}",
            trim.gain
        );

        // A hot feed is reduced, down to the limit.
        let mut trim = trim_at_1khz();
        feed(&mut trim, 1.5, 15000);
        let min = AutoTrim::db_to_linear(AutoTrim::MIN_GAIN_DB);
        assert!(
            trim.gain < 0.75 && trim.gain >= min - 0.01,
            "a hot feed should be reduced but not below {min:.2}, got {:.3}",
            trim.gain
        );
    }

    /// A signal that touches full scale is not clipping; one that sits
    /// there has been through a converter that ran out of range. Runs are
    /// counted per channel, and any one channel clipping is enough.
    #[test]
    fn a_run_at_full_scale_on_any_channel_is_clipping() {
        let mono = channels(1);
        let mut buffer = vec![0.5_f32; 64];
        buffer[10] = 1.0;
        buffer[30] = -1.0;
        assert!(
            !buffer_clips(&buffer, mono),
            "isolated samples at full scale are not clipping"
        );
        buffer[20..22].fill(1.0);
        assert!(
            !buffer_clips(&buffer, mono),
            "two samples at full scale are not a run"
        );
        buffer[40..43].fill(-1.0);
        assert!(
            buffer_clips(&buffer, mono),
            "three samples pinned at negative full scale are a run"
        );

        let stereo = channels(2);
        for channel in 0..2 {
            let mut buffer = vec![0.5_f32; 128];
            for frame in 10..18 {
                buffer[2 * frame + channel] = 1.0;
            }
            assert!(
                buffer_clips(&buffer, stereo),
                "channel {channel} pinned for eight frames is clipping, whatever the other does"
            );
        }
        let mut buffer = vec![0.5_f32; 128];
        buffer[40..44].fill(1.0);
        assert!(
            !buffer_clips(&buffer, stereo),
            "both channels at full scale for two frames is two samples per channel, not a run"
        );
    }

    /// The clip indicator lights on a buffer that clipped and stays lit for
    /// the hold time of clean input after it.
    #[test]
    fn clip_indicator_holds_after_a_clipped_buffer() {
        // 48 frames at 48 kHz: each buffer is 1 ms.
        let mut processor = Processor::new(
            ProcessorSettings::default(),
            48000,
            channels(1),
            test_producers(),
        );
        let clean = vec![0.5_f32; 48];
        let mut clipped = clean.clone();
        clipped[20..28].fill(1.0);

        processor.process(&clean);
        assert!(
            !processor.input_meter.clip_lit(),
            "clean input leaves the indicator dark"
        );
        processor.process(&clipped);
        assert!(
            processor.input_meter.clip_lit(),
            "a clipped buffer lights the indicator"
        );
        for _ in 0..299 {
            processor.process(&clean);
        }
        assert!(
            processor.input_meter.clip_lit(),
            "299 ms of clean input after clipping is within the hold"
        );
        for _ in 0..2 {
            processor.process(&clean);
        }
        assert!(
            !processor.input_meter.clip_lit(),
            "301 ms of clean input after clipping is past the hold"
        );
    }

    /// The meter handle stops upgrading once its processor is dropped.
    #[test]
    fn input_meter_lives_as_long_as_its_processor() {
        let processor = Processor::new(
            ProcessorSettings::default(),
            48000,
            channels(1),
            test_producers(),
        );
        let meter = processor.input_meter();
        assert!(
            meter.upgrade().is_some(),
            "the processor keeps its meter alive"
        );
        drop(processor);
        assert!(
            meter.upgrade().is_none(),
            "nothing but the processor keeps its meter alive"
        );
    }

    /// Helper: feed a processor one mono signal in 48-sample buffers for the
    /// given duration. Returns the final envelope value.
    fn run_processor(
        settings: &ProcessorSettings,
        sample_rate: u32,
        duration_secs: f32,
        sample: impl Fn(f32) -> f32,
    ) -> f32 {
        let buffer_size = 48;
        let total_samples = (duration_secs * sample_rate as f32) as usize;
        let mut processor =
            Processor::new(settings.clone(), sample_rate, channels(1), test_producers());

        let mut idx = 0;
        while idx < total_samples {
            let end = (idx + buffer_size).min(total_samples);
            let buffer: Vec<f32> = (idx..end)
                .map(|i| sample(i as f32 / sample_rate as f32))
                .collect();
            processor.process(&buffer);
            idx = end;
        }

        settings.envelope.get()
    }
}
