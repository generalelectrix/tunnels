//! A multi-channel audio processor that derives a single envelope from its input.
//! Provides lowpass filterting and configurable amplitude envelope.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::AudioProcessorSettings;
use audio_processor_traits::{simple_processor::MonoAudioProcessor, AtomicF32, AudioContext};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::debug;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ProcessorSettingsInner {
    pub envelope: AtomicF32,
    pub filter_cutoff: AtomicF32,    // Hz
    pub envelope_attack: AtomicF32,  // sec
    pub envelope_release: AtomicF32, // sec
}

impl ProcessorSettingsInner {
    const DEFAULT_FILTER_CUTOFF: f32 = 200.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.01;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.1;

    pub fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
    }
}

impl Default for ProcessorSettingsInner {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            filter_cutoff: AtomicF32::new(Self::DEFAULT_FILTER_CUTOFF),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
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
    filters: Vec<FilterProcessor<f32>>,
    envelopes: Vec<EnvelopeFollowerProcessor>,
    context: AudioContext,
}

impl Processor {
    pub fn new(handle: ProcessorSettings, sample_rate: u32, channel_count: usize) -> Self {
        let mut context: AudioContext = AudioProcessorSettings {
            sample_rate: sample_rate as f32,
            input_channels: channel_count,
            output_channels: channel_count, // unused
            ..Default::default()
        }
        .into();
        let settings = context.settings;

        let mut filters = vec![];
        let mut envelopes = vec![];

        let filter_cutoff = handle.filter_cutoff.get();
        let envelope_attack = handle.envelope_attack.get();
        let envelope_release = handle.envelope_release.get();

        for _ in 0..settings.input_channels {
            let mut filter = FilterProcessor::new(FilterType::LowPass);
            filter.set_cutoff(filter_cutoff);
            filter.m_prepare(&mut context);
            filters.push(filter);

            let mut envelope = EnvelopeFollowerProcessor::new(
                Duration::from_secs_f32(envelope_attack),
                Duration::from_secs_f32(envelope_release),
            );
            envelope.m_prepare(&mut context);

            envelopes.push(envelope);
        }

        Self {
            filter_cutoff,
            envelope_attack,
            envelope_release,
            settings: handle,
            channel_count: settings.input_channels,
            filters,
            envelopes,
            context,
        }
    }

    /// Load current parameters and update filters/envelopes if they have changed.
    fn maybe_update_parameters(&mut self) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            debug!("Updating filter cutoff to {}", new_filter_cutoff);
            self.filter_cutoff = new_filter_cutoff;
            for filter in self.filters.iter_mut() {
                filter.set_cutoff(new_filter_cutoff);
            }
        }
        let new_envelope_attack = self.settings.envelope_attack.get();
        let new_envelope_release = self.settings.envelope_release.get();
        if new_envelope_attack != self.envelope_attack
            || new_envelope_release != self.envelope_release
        {
            debug!(
                "Updating envelope parameters to {}, {}",
                new_envelope_attack, new_envelope_release
            );
            self.envelope_attack = new_envelope_attack;
            self.envelope_release = new_envelope_release;
            let attack = Duration::from_secs_f32(new_envelope_attack);
            let release = Duration::from_secs_f32(new_envelope_release);
            for envelope in self.envelopes.iter_mut() {
                let handle = envelope.handle();
                handle.set_attack(attack);
                handle.set_release(release);
            }
        }
    }
}

impl Processor {
    /// Process a buffer of interleaved audio data.
    pub fn process(&mut self, interleaved_buffer: &[f32]) {
        self.maybe_update_parameters();

        for frame in interleaved_buffer.chunks(self.channel_count) {
            for (channel_idx, sample) in frame.iter().enumerate() {
                let sample = self.filters[channel_idx].m_process(&mut self.context, *sample);
                let envelope = &mut self.envelopes[channel_idx];
                envelope.m_process(&mut self.context, sample);
            }
        }

        let mean_envelope = self
            .envelopes
            .iter()
            .map(|envelope| envelope.handle().state())
            .sum::<f32>()
            / self.channel_count as f32;

        self.settings.envelope.set(mean_envelope);
    }
}
