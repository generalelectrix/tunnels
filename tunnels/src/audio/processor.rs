//! A multi-channel audio processor that derives a single envelope from its input.
//!
//! Processing chain per channel:
//!   lowpass (band isolation) → Hilbert |z(t)| (rectification) → fast envelope → slow envelope
//!
//! The Hilbert transform provides clean rectification without carrier ripple.
//! The two-stage envelope follower separates peak tracking (fast) from
//! smoothing (slow), giving better transient response than a single stage.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{AtomicF32, AudioContext, simple_processor::MonoAudioProcessor};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::debug;
use std::sync::Arc;
use std::time::Duration;

use super::hilbert::HilbertTransform;
use super::ring_buffer::SignalRingBuffer;

/// 16384 slots ≈ 16 seconds of history at ~1kHz buffer rate.
const ENVELOPE_HISTORY_CAPACITY: usize = 16384;

/// Fast envelope follower: catches every peak within a cycle.
const FAST_ATTACK: Duration = Duration::from_millis(1);
const FAST_RELEASE: Duration = Duration::new(0, 4_000_000); // 4ms

pub struct ProcessorSettingsInner {
    /// Current envelope value for the show loop.
    pub envelope: AtomicF32,
    pub filter_cutoff: AtomicF32,    // Hz
    pub envelope_attack: AtomicF32,  // sec (slow stage)
    pub envelope_release: AtomicF32, // sec (slow stage)
    /// Input signal gain multiplier (linear scale).
    pub gain: AtomicF32,
    /// Symmetric output smoothing time constant (seconds). 0 = disabled.
    pub output_smoothing: AtomicF32,

    // === Ring buffers for visualization ===
    /// Envelope after two-stage follower (before output smoothing).
    pub envelope_history: SignalRingBuffer,
    /// Per-buffer input peak amplitude.
    pub input_peak_history: SignalRingBuffer,
    /// Envelope after symmetric output smoothing.
    pub smoothed_history: SignalRingBuffer,
}

impl ProcessorSettingsInner {
    const DEFAULT_FILTER_CUTOFF: f32 = 200.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.010;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.050;
    /// Default output smoothing: 8ms (~2 render frames at 240fps).
    const DEFAULT_OUTPUT_SMOOTHING: f32 = 0.008;

    pub fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
        self.output_smoothing.set(Self::DEFAULT_OUTPUT_SMOOTHING);
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
            envelope_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            input_peak_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
            smoothed_history: SignalRingBuffer::new(ENVELOPE_HISTORY_CAPACITY),
        }
    }
}

pub type ProcessorSettings = Arc<ProcessorSettingsInner>;

pub struct Processor {
    settings: ProcessorSettings,
    filter_cutoff: f32,
    envelope_attack: f32,
    envelope_release: f32,
    channel_count: usize,
    context: AudioContext,

    filters: Vec<FilterProcessor<f32>>,
    hilberts: Vec<HilbertTransform>,
    fast_envelopes: Vec<EnvelopeFollowerProcessor>,
    slow_envelopes: Vec<EnvelopeFollowerProcessor>,

    /// Symmetric one-pole smoother state (one value, not per-channel —
    /// applied after the per-channel envelopes are averaged).
    smoothed_value: f32,
    /// Cached smoother coefficient, recomputed when output_smoothing changes.
    smooth_coeff: f32,
    smooth_time: f32,
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
    pub fn new(handle: ProcessorSettings, sample_rate: u32, channel_count: usize) -> Self {
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

        let mut filters = vec![];
        let mut hilberts = vec![];
        let mut fast_envelopes = vec![];
        let mut slow_envelopes = vec![];

        for _ in 0..n {
            let mut filter = FilterProcessor::new(FilterType::LowPass);
            filter.set_cutoff(filter_cutoff);
            filter.m_prepare(&mut context);
            filters.push(filter);

            hilberts.push(HilbertTransform::new());
            fast_envelopes.push(make_envelope(&mut context, FAST_ATTACK, FAST_RELEASE));
            slow_envelopes.push(make_envelope(&mut context, slow_attack, slow_release));
        }

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: n,
            context,
            filters,
            hilberts,
            fast_envelopes,
            slow_envelopes,
            smoothed_value: 0.0,
            smooth_coeff: 0.0,
            smooth_time: 0.0,
        }
    }

    /// Compute the one-pole coefficient from a time constant and the
    /// actual buffer update rate (sample_rate / frames_per_buffer).
    fn compute_smooth_coeff(time_secs: f32, update_rate: f32) -> f32 {
        if time_secs <= 0.0 || update_rate <= 0.0 {
            return 0.0; // disabled
        }
        (-1.0 / (time_secs * update_rate)).exp()
    }

    fn maybe_update_parameters(&mut self) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            debug!("Updating filter cutoff to {new_filter_cutoff}");
            self.filter_cutoff = new_filter_cutoff;
            for filter in &mut self.filters {
                filter.set_cutoff(new_filter_cutoff);
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
            for env in &mut self.slow_envelopes {
                env.handle().set_attack(attack);
                env.handle().set_release(release);
            }
        }

    }

    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        if interleaved_buffer.is_empty() {
            return;
        }

        self.maybe_update_parameters();

        // Recompute smoothing coefficient if the time constant changed,
        // using the actual buffer size to determine the update rate.
        let new_smooth = self.settings.output_smoothing.get();
        if new_smooth != self.smooth_time {
            self.smooth_time = new_smooth;
            let frames = interleaved_buffer.len() / self.channel_count.max(1);
            if frames > 0 {
                let update_rate = self.context.settings.sample_rate / frames as f32;
                self.smooth_coeff = Self::compute_smooth_coeff(new_smooth, update_rate);
            }
        }

        let mut input_peak: f32 = 0.0;
        let gain = self.settings.gain.get();

        for frame in interleaved_buffer.chunks(self.channel_count) {
            for (ch, raw_sample) in frame.iter().enumerate() {
                let sample = *raw_sample * gain;
                input_peak = input_peak.max(sample.abs());

                // lowpass → Hilbert |z(t)| → fast envelope → slow envelope
                let filtered = self.filters[ch].m_process(&mut self.context, sample);
                let amplitude = self.hilberts[ch].envelope(filtered as f64) as f32;
                self.fast_envelopes[ch].m_process(&mut self.context, amplitude);
                let fast_val = self.fast_envelopes[ch].handle().state();
                self.slow_envelopes[ch].m_process(&mut self.context, fast_val);
            }
        }

        let ch_count = self.channel_count as f32;
        let envelope = self
            .slow_envelopes
            .iter()
            .map(|e| e.handle().state())
            .sum::<f32>()
            / ch_count;

        // Symmetric one-pole smoothing: y[n] = coeff * y[n-1] + (1 - coeff) * x[n]
        let coeff = self.smooth_coeff;
        self.smoothed_value = coeff * self.smoothed_value + (1.0 - coeff) * envelope;

        self.settings.envelope.set(envelope);
        self.settings.envelope_history.push(envelope);
        self.settings.input_peak_history.push(input_peak);
        self.settings.smoothed_history.push(self.smoothed_value);
    }
}
