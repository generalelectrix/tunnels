//! A multi-channel audio processor that derives per-band envelopes from its input.
//!
//! Every input channel first passes through an automatic trim and a DC
//! blocker. Processing chains:
//!   Lowpass: per-channel lowpass → Hilbert |z(t)| → fast envelope → slow envelope
//!   Wavelet: mono D4 decomposition → per-band Hilbert → fast → slow envelope
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
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            norm_floor_halflife: AtomicF32::new(10.0),
            norm_ceiling_halflife: AtomicF32::new(5.0),
            norm_floor_mode: AtomicTrackingMode::new(TrackingMode::Average),
            norm_ceiling_mode: AtomicTrackingMode::new(TrackingMode::Limit),
            active_band: AtomicU32::new(0),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// A slow automatic gain that brings the input's peaks toward a fixed target,
/// within limits, so the level the interface is set to does not decide the
/// level every band works from. It holds still on silence.
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
    /// mastered level; past that it would mostly be lifting the interface's
    /// own noise.
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
    /// EMA coefficients for average-mode floor tracking.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// EMA coefficient for limit-mode floor (slow rise from minimum).
    floor_limit_rise_coeff: f32,
    /// EMA coefficient for ceiling decay.
    ceiling_fall_coeff: f32,
}

impl AdaptiveNormalizer {
    /// Minimum range between floor and ceiling. This caps the maximum gain
    /// at 1/MIN_RANGE. With normalized input (~unity), 0.333 = max 3x gain.
    const MIN_RANGE: f32 = 0.333;

    fn new() -> Self {
        Self {
            floor: 0.0,
            ceiling: 0.001,
            floor_rise_coeff: 0.999,
            floor_fall_coeff: 0.99,
            floor_limit_rise_coeff: 0.999,
            ceiling_fall_coeff: 0.999,
        }
    }

    fn set_params(
        &mut self,
        floor_halflife: Duration,
        ceiling_halflife: Duration,
        update_rate: f32,
    ) {
        if update_rate <= 0.0 {
            return;
        }
        // Average mode: slow rise, faster fall.
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff = halflife_to_coeff(floor_halflife.mul_f32(0.2), update_rate);
        // Limit mode: instant drop to min, slow rise back.
        self.floor_limit_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        // Ceiling: instant attack, decays at ceiling halflife.
        self.ceiling_fall_coeff = halflife_to_coeff(ceiling_halflife, update_rate);
    }

    #[inline]
    fn process(
        &mut self,
        envelope: f32,
        floor_mode: TrackingMode,
        ceiling_mode: TrackingMode,
    ) -> f32 {
        // Update floor.
        match floor_mode {
            TrackingMode::Average => {
                let coeff = if envelope > self.floor {
                    self.floor_rise_coeff
                } else {
                    self.floor_fall_coeff
                };
                self.floor = coeff * self.floor + (1.0 - coeff) * envelope;
            }
            TrackingMode::Limit => {
                if envelope < self.floor {
                    self.floor = envelope; // instant drop to minimum
                } else {
                    self.floor = self.floor_limit_rise_coeff * self.floor
                        + (1.0 - self.floor_limit_rise_coeff) * envelope;
                }
            }
        }

        // Update ceiling.
        match ceiling_mode {
            TrackingMode::Average => {
                // Symmetric EMA — same speed up and down.
                self.ceiling = self.ceiling_fall_coeff * self.ceiling
                    + (1.0 - self.ceiling_fall_coeff) * envelope;
            }
            TrackingMode::Limit => {
                if envelope > self.ceiling {
                    self.ceiling = envelope; // instant rise to maximum
                } else {
                    self.ceiling = self.ceiling_fall_coeff * self.ceiling
                        + (1.0 - self.ceiling_fall_coeff) * envelope;
                }
            }
        }

        let range = (self.ceiling - self.floor).max(Self::MIN_RANGE);
        ((envelope - self.floor) / range).clamp(0.0, 1.0)
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
                normalizer: AdaptiveNormalizer::new(),
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
            lowpass_normalizer: AdaptiveNormalizer::new(),
            smooth_coeff: SmootherCoeff::default(),
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

        // Normalization params (shared between lowpass and wavelet normalizers).
        let floor_hl = secs_to_duration(self.settings.norm_floor_halflife.get());
        let ceil_hl = secs_to_duration(self.settings.norm_ceiling_halflife.get());
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
