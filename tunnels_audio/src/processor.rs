//! A multi-channel audio processor that derives per-band envelopes from its input.
//!
//! Processing chains:
//! On the mono mix of the input channels: undecimated D4 decomposition into
//! octave bands and the sub-bass residual → per-band Hilbert |z(t)| →
//! fast envelope → slow envelope → smoother → adaptive normalizer.
//!
//! Output: `NUM_OUTPUT_BANDS` normalized bands, the residual and the octaves
//! below the one under Nyquist, selectable via `active_band`.
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::hilbert::HilbertTransform;
use crate::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};
use crate::wavelet::WaveletDecomposition;

/// Fast envelope follower half-lives in seconds: catches every peak within
/// a cycle.
const FAST_ATTACK: f32 = 0.001;
const FAST_RELEASE: f32 = 0.004;

/// An `f32` shared between threads, stored as its bit pattern.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Number of output bands, in ascending frequency order from the sub-bass
/// residual.
pub const NUM_OUTPUT_BANDS: usize = crate::wavelet::NUM_BANDS;

/// Ring buffer capacity: ~16 seconds of history at ~1kHz buffer rate.
pub const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// The frequency each output band starts at: band 0 is the residual below
/// band 1, each band after that is an octave, and band `NUM_OUTPUT_BANDS`
/// names the top edge of the highest one.
fn output_band_start(band: usize, sample_rate: u32) -> f32 {
    debug_assert!((1..=NUM_OUTPUT_BANDS).contains(&band));
    sample_rate as f32 / (1_u32 << (NUM_OUTPUT_BANDS - band + 2)) as f32
}

/// A frequency as an operator reads it: "94", "1.5k", "12k".
fn format_hz(hz: f32) -> String {
    if hz < 1000.0 {
        return format!("{hz:.0}");
    }
    let k = hz / 1000.0;
    if (k - k.round()).abs() < 0.05 {
        format!("{k:.0}k")
    } else {
        format!("{k:.1}k")
    }
}

/// What each output band covers at `sample_rate`, in output order. The bands
/// are octaves of the sample rate, so they move with it.
pub fn output_band_labels(sample_rate: u32) -> [String; NUM_OUTPUT_BANDS] {
    std::array::from_fn(|band| {
        let start = output_band_start(band.max(1), sample_rate);
        if band == 0 {
            format!("<{} Hz", format_hz(start))
        } else {
            format!(
                "{}-{} Hz",
                format_hz(start),
                format_hz(output_band_start(band + 1, sample_rate))
            )
        }
    })
}

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
    /// Ceiling tracking half-life, in seconds of music at
    /// `REFERENCE_MOTION_RATE`. The ceiling's memory is really a quantity of
    /// envelope motion, so quiet or still material stretches these seconds
    /// and a silent band holds the ceiling indefinitely.
    pub norm_ceiling_halflife: AtomicF32,

    /// Which band feeds `envelope`: 0 = sub-bass residual, the rest octaves.
    pub active_band: AtomicU32,
}

impl ProcessorSettingsInner {
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;
    /// Floor half-life: slow enough that a bass line is above its own bed,
    /// fast enough that a held tone stops being news within a phrase or two.
    pub const DEFAULT_FLOOR_HALFLIFE: f32 = 10.0;
    /// Ceiling half-life of about one four-bar phrase. A beat moves the log
    /// envelope about 8 nepers whatever the tempo, so 8 s at
    /// `REFERENCE_MOTION_RATE` is ~52 nepers, and each band is measured
    /// against the loudest thing in the phrase it is part of.
    pub const DEFAULT_CEILING_HALFLIFE: f32 = 8.0;

    pub fn reset_defaults(&self) {
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
        self.gain.set(1.0);
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
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
            gain: AtomicF32::new(1.0),
            output_smoothing: AtomicF32::new(Self::DEFAULT_OUTPUT_SMOOTHING),
            norm_floor_halflife: AtomicF32::new(Self::DEFAULT_FLOOR_HALFLIFE),
            norm_ceiling_halflife: AtomicF32::new(Self::DEFAULT_CEILING_HALFLIFE),
            active_band: AtomicU32::new(0),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

/// Nepers per second the log envelope of a band moves on music: the median
/// over five tracks and seven bands, where the spread is 4.0 to 10.1. The
/// ceiling's memory is a quantity of envelope motion; this rate is what
/// converts it to a half-life in seconds for the operator's benefit.
pub const REFERENCE_MOTION_RATE: f32 = 6.5;

/// One-pole EMA coefficient that halves the distance to the target every
/// `halflife_secs` at `update_rate` updates per second. A non-positive
/// half-life means no smoothing.
fn halflife_to_coeff(halflife_secs: f32, update_rate: f32) -> f32 {
    if halflife_secs <= 0.0 || update_rate <= 0.0 {
        return 0.0;
    }
    (-f32::ln(2.0) / (halflife_secs * update_rate)).exp()
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
struct NormalizerParams {
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
    floor_halflife: f32,
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
            floor_halflife: 0.0,
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
        let floor_halflife = settings.norm_floor_halflife.get();
        if update_rate <= 0.0
            || (floor_halflife == self.floor_halflife && update_rate == self.update_rate)
        {
            return;
        }
        self.floor_halflife = floor_halflife;
        self.update_rate = update_rate;
        self.floor_rise_coeff = halflife_to_coeff(floor_halflife, update_rate);
        self.floor_fall_coeff =
            halflife_to_coeff(floor_halflife * Self::FLOOR_FALL_RATIO, update_rate);
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
    /// Follows the envelope slowly upward and faster downward: the bed the
    /// output is measured from.
    floor: AsymmetricOnePole,
    ceiling: f32,
    /// Log envelope on the previous update, clamped at the noise gate.
    prev_log_envelope: f32,
}

impl AdaptiveNormalizer {
    fn new(tuning: &NormalizerTuning) -> Self {
        Self {
            floor: AsymmetricOnePole::default(),
            ceiling: 0.0,
            prev_log_envelope: tuning.noise_gate.ln(),
        }
    }

    #[inline]
    fn process(&mut self, envelope: f32, p: &NormalizerParams) -> f32 {
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
struct AsymmetricOnePole {
    rise: f32,
    fall: f32,
    state: f32,
}

impl AsymmetricOnePole {
    fn new(rise: f32, fall: f32) -> Self {
        Self {
            rise,
            fall,
            state: 0.0,
        }
    }

    #[inline]
    fn step(&mut self, input: f32) -> f32 {
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

/// Envelope chain for one output band:
/// Hilbert |z(t)| → fast envelope → slow envelope → smoother → normalizer.
struct BandChain {
    hilbert: HilbertTransform,
    fast_envelope: AsymmetricOnePole,
    slow_envelope: AsymmetricOnePole,
    smoother: OnePoleSmoother,
    normalizer: AdaptiveNormalizer,
}

impl BandChain {
    /// `sample_rate` in Hz; the slow follower's attack and release are
    /// half-lives in seconds.
    fn new(
        sample_rate: f32,
        slow_attack: f32,
        slow_release: f32,
        tuning: &NormalizerTuning,
    ) -> Self {
        Self {
            hilbert: HilbertTransform::new(),
            fast_envelope: AsymmetricOnePole::new(
                halflife_to_coeff(FAST_ATTACK, sample_rate),
                halflife_to_coeff(FAST_RELEASE, sample_rate),
            ),
            slow_envelope: AsymmetricOnePole::new(
                halflife_to_coeff(slow_attack, sample_rate),
                halflife_to_coeff(slow_release, sample_rate),
            ),
            smoother: OnePoleSmoother::default(),
            normalizer: AdaptiveNormalizer::new(tuning),
        }
    }

    /// Run one band-limited sample through the envelope followers.
    #[inline]
    fn process_sample(&mut self, sample: f32) {
        let amplitude = self.hilbert.envelope(f64::from(sample)) as f32;
        let fast = self.fast_envelope.step(amplitude);
        self.slow_envelope.step(fast);
    }

    /// Finish a buffer: smooth the slow envelope and normalize it, returning
    /// the band's output.
    fn finish(&mut self, smooth_coeff: f32, norm: &NormalizerParams) -> f32 {
        let smoothed = self.smoother.update(smooth_coeff, self.slow_envelope.state);
        self.normalizer.process(smoothed, norm)
    }

    /// Set the slow follower's attack and release half-lives in seconds.
    fn set_slow_envelope(&mut self, attack: f32, release: f32, sample_rate: f32) {
        self.slow_envelope.rise = halflife_to_coeff(attack, sample_rate);
        self.slow_envelope.fall = halflife_to_coeff(release, sample_rate);
    }

    fn stages(&self) -> BandStages {
        BandStages {
            smoothed: self.smoother.state,
            floor: self.normalizer.floor.state,
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
    /// Interleaved channels per frame, never zero.
    channel_count: usize,
    sample_rate: f32,

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

impl Processor {
    pub fn new(
        handle: ProcessorSettings,
        sample_rate: u32,
        channel_count: usize,
        envelope_producers: [EnvelopeProducer; NUM_OUTPUT_BANDS],
    ) -> Self {
        let sample_rate = sample_rate as f32;
        let envelope_attack = handle.envelope_attack.get();
        let envelope_release = handle.envelope_release.get();
        let tuning = NormalizerTuning::DEFAULT;
        let bands = std::array::from_fn(|_| {
            BandChain::new(sample_rate, envelope_attack, envelope_release, &tuning)
        });

        Self {
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: channel_count.max(1),
            sample_rate,
            envelope_producers,
            wavelet: WaveletDecomposition::new(),
            bands,
            smooth_coeff: SmootherCoeff::default(),
            norm_params: NormalizerParams::new(tuning),
        }
    }

    /// Replace the normalizer tuning and restart every band's normalizer
    /// from its initial state.
    pub fn set_normalizer_tuning(&mut self, tuning: NormalizerTuning) {
        self.norm_params = NormalizerParams::new(tuning);
        for band in &mut self.bands {
            band.normalizer = AdaptiveNormalizer::new(&tuning);
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
            self.envelope_attack = new_attack;
            self.envelope_release = new_release;
            for band in &mut self.bands {
                band.set_slow_envelope(new_attack, new_release, self.sample_rate);
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

        let frames = interleaved_buffer.len() / self.channel_count;
        if frames == 0 {
            return;
        }
        let update_rate = self.sample_rate / frames as f32;

        self.maybe_update_parameters(update_rate);

        let gain = self.settings.gain.get();
        let ch_count_f = self.channel_count as f32;

        for frame in interleaved_buffer.chunks(self.channel_count) {
            // Both paths run on the mono mix.
            let mono = frame.iter().sum::<f32>() / ch_count_f * gain;

            let bands = &mut self.bands;
            self.wavelet.push(mono, |band, sample| {
                bands[band].process_sample(sample);
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
