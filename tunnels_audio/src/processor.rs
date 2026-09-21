//! A multi-channel audio processor that derives per-band envelopes from its input.
//!
//! Processing chains:
//! On the mono mix of the input channels: undecimated D4 decomposition into
//! seven octave bands and the sub-bass residual → per-band Hilbert |z(t)| →
//! fast envelope → slow envelope → smoother → adaptive normalizer.
//!
//! Output: 8 normalized bands (residual + 7 octaves), selectable via `active_band`.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{AtomicF32, AudioContext, simple_processor::MonoAudioProcessor};
use log::debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::hilbert::HilbertTransform;
use crate::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};
use crate::wavelet::{NUM_LEVELS, WaveletDecomposition};

/// Fast envelope follower: catches every peak within a cycle.
const FAST_ATTACK: Duration = Duration::from_millis(1);
const FAST_RELEASE: Duration = Duration::new(0, 4_000_000); // 4ms

/// Number of output bands: the sub-bass residual + 7 octave bands.
pub const NUM_OUTPUT_BANDS: usize = 8;

/// Ring buffer capacity: ~16 seconds of history at ~1kHz buffer rate.
pub const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// Band labels in frequency-ascending output order (index 0 = sub-bass residual).
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
    pub envelope_attack: AtomicF32,  // sec (slow stage)
    pub envelope_release: AtomicF32, // sec (slow stage)
    /// Input signal gain multiplier (linear scale).
    pub gain: AtomicF32,
    /// Symmetric output smoothing time constant (seconds). 0 = disabled.
    pub output_smoothing: AtomicF32,

    /// Floor tracking half-life in seconds (slow — adapts to ambient level).
    pub norm_floor_halflife: AtomicF32,
    pub norm_floor_mode: AtomicTrackingMode,

    /// Which band feeds `envelope`: 0 = sub-bass residual, 1-7 = octave bands.
    pub active_band: AtomicU32,
}

impl ProcessorSettingsInner {
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;

    pub fn reset_defaults(&self) {
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
        self.gain.set(1.0);
        self.active_band.store(0, Ordering::Relaxed);
        self.norm_floor_halflife.set(10.0);
        self.norm_floor_mode
            .store(TrackingMode::Average, Ordering::Relaxed);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
            gain: AtomicF32::new(1.0),
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            norm_floor_halflife: AtomicF32::new(10.0),
            norm_floor_mode: AtomicTrackingMode::new(TrackingMode::Average),
            active_band: AtomicU32::new(0),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// One-pole EMA coefficient that halves the distance to the target every
/// `halflife_secs` at `update_rate` updates per second. A non-positive
/// half-life means no smoothing.
fn halflife_to_coeff(halflife_secs: f32, update_rate: f32) -> f32 {
    if halflife_secs <= 0.0 || update_rate <= 0.0 {
        return 0.0;
    }
    (-f32::ln(2.0) / (halflife_secs * update_rate)).exp()
}

/// How the normalizer floor tracks the envelope.
/// - Average: asymmetric EMA tracking the general level
/// - Limit: drops instantly to the minimum and rises slowly from it
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
    /// EMA coefficients for average-mode floor tracking.
    floor_rise_coeff: f32,
    floor_fall_coeff: f32,
    /// EMA coefficient for limit-mode floor (slow rise from minimum).
    floor_limit_rise_coeff: f32,
    /// The inputs the coefficients were derived from.
    floor_halflife: f32,
    update_rate: f32,
}

impl NormalizerParams {
    fn new() -> Self {
        Self {
            floor_mode: TrackingMode::Average,
            floor_rise_coeff: 0.0,
            floor_fall_coeff: 0.0,
            floor_limit_rise_coeff: 0.0,
            floor_halflife: 0.0,
            update_rate: 0.0,
        }
    }

    /// Refresh from the settings for the given update rate. Safe to call
    /// every buffer.
    fn refresh(&mut self, settings: &ProcessorSettingsInner, update_rate: f32) {
        self.floor_mode = settings.norm_floor_mode.load(Ordering::Relaxed);
        let floor_halflife = settings.norm_floor_halflife.get();
        if update_rate <= 0.0
            || (floor_halflife == self.floor_halflife && update_rate == self.update_rate)
        {
            return;
        }
        self.floor_halflife = floor_halflife;
        self.update_rate = update_rate;
        // Average mode: slow rise, faster fall.
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff = halflife_to_coeff(floor_halflife * 0.2, update_rate);
        // Limit mode: instant drop to min, slow rise back.
        self.floor_limit_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
    }
}

/// Adaptive envelope normalizer: tracks a floor and ceiling,
/// outputs `(envelope - floor) / (ceiling - floor)` clamped to [0, 1].
///
/// The ceiling is a peak follower whose decay is clocked by the envelope's
/// own motion rather than by time: it rises to any envelope above it at once
/// and forgets `CEILING_FORGET` nepers for every neper the log envelope
/// moves, up or down. A hit therefore costs the ceiling the same whether
/// the music is fast or slow, a level drop is forgotten within a few hits
/// at any tempo, and a pause or a held tone — no motion — leaves the
/// ceiling where it was.
struct AdaptiveNormalizer {
    floor: f32,
    ceiling: f32,
    /// Log envelope on the previous update, clamped at the gate.
    prev_log_envelope: f32,
    /// Envelope level below which the band outputs zero.
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
    /// Nepers the ceiling decays per neper of log-envelope motion.
    const CEILING_FORGET: f32 = 0.1;

    fn new() -> Self {
        Self {
            floor: 0.0,
            ceiling: 0.0,
            prev_log_envelope: Self::NOISE_GATE.ln(),
            gate: Self::NOISE_GATE,
        }
    }

    #[inline]
    fn process(&mut self, envelope: f32, p: &NormalizerParams) -> f32 {
        // Update ceiling: instant attack, decay per unit of envelope motion.
        let log_envelope = envelope.max(self.gate).ln();
        let motion = (log_envelope - self.prev_log_envelope).abs();
        self.prev_log_envelope = log_envelope;
        self.ceiling = envelope.max(self.ceiling * (-Self::CEILING_FORGET * motion).exp());

        // Update floor. It never exceeds the ceiling.
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
        if range <= 0.0 {
            // Nothing has formed a ceiling yet; the signal is above anything seen.
            return 1.0;
        }
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
    fn new(context: &mut AudioContext, slow_attack: Duration, slow_release: Duration) -> Self {
        Self {
            hilbert: HilbertTransform::new(),
            fast_envelope: make_envelope(context, FAST_ATTACK, FAST_RELEASE),
            slow_envelope: make_envelope(context, slow_attack, slow_release),
            smoother: OnePoleSmoother::default(),
            normalizer: AdaptiveNormalizer::new(),
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
    envelope_attack: f32,
    envelope_release: f32,
    channel_count: usize,
    context: AudioContext,

    /// Envelope ring buffer producers — one per output band.
    envelope_producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],

    /// Wavelet decomposition feeding every band.
    wavelet: WaveletDecomposition,
    /// One envelope chain per output band, in output order: 0 is the
    /// residual sub-bass band, 1..NUM_OUTPUT_BANDS the octave bands in
    /// ascending frequency order.
    bands: [BandChain; NUM_OUTPUT_BANDS],
    /// Cached smoother coefficient shared across every band's smoother.
    smooth_coeff: SmootherCoeff,
    /// Normalizer coefficients shared across every band's normalizer.
    norm_params: NormalizerParams,
}

/// The output band that wavelet level `level` feeds: levels count down from
/// the highest octave, output bands count up from the residual.
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

        let envelope_attack = handle.envelope_attack.get();
        let envelope_release = handle.envelope_release.get();
        let slow_attack = Duration::from_secs_f32(envelope_attack);
        let slow_release = Duration::from_secs_f32(envelope_release);

        let bands =
            std::array::from_fn(|_| BandChain::new(&mut context, slow_attack, slow_release));

        Self {
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: n,
            context,
            envelope_producers,
            wavelet: WaveletDecomposition::new(),
            bands,
            smooth_coeff: SmootherCoeff::default(),
            norm_params: NormalizerParams::new(),
        }
    }

    /// Read an output band's intermediate stage values. Index 0 is the
    /// sub-bass residual; 1..NUM_OUTPUT_BANDS are the octave bands in
    /// ascending frequency order.
    pub fn band_stages(&self, output_band: usize) -> Option<BandStages> {
        self.bands.get(output_band).map(BandChain::stages)
    }

    fn maybe_update_parameters(&mut self, update_rate: f32) {
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

        let gain = self.settings.gain.get();
        let ch_count_f = self.channel_count as f32;

        for frame in interleaved_buffer.chunks(self.channel_count) {
            // Both paths run on the mono mix.
            let mono = frame.iter().sum::<f32>() / ch_count_f * gain;

            let bands = &mut self.bands;
            let ctx = &mut self.context;
            self.wavelet.push(mono, |level, sample| {
                bands[output_band_of_level(level)].process_sample(sample, ctx);
            });
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

    #[test]
    fn processor_produces_envelope_from_sine() {
        let settings = ProcessorSettings::default();
        let envelope = run_processor_with_sine(0.7, 100.0, 1.0, &settings);
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
}
