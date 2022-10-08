//! A multi-channel audio processor that derives a single envelope from its input.
//! Provides lowpass filterting and configurable amplitude envelope.
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::{AtomicF32, AudioProcessorSettings, SimpleAudioProcessor};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use log::{info, warn};
use std::sync::Arc;
use std::time::Duration;

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
}

impl Processor {
    pub fn new(handle: ProcessorSettings) -> Self {
        Self {
            filter_cutoff: handle.filter_cutoff.get(),
            envelope_attack: handle.envelope_attack.get(),
            envelope_release: handle.envelope_release.get(),
            settings: handle,
            channel_count: 0,
            filters: Vec::new(),
            envelopes: Vec::new(),
        }
    }

    /// Load current parameters and update filters/envelopes if they have changed.
    fn maybe_update_parameters(&mut self) {
        let new_filter_cutoff = self.settings.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            info!("Updating filter cutoff to {}", new_filter_cutoff);
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
            info!(
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

impl SimpleAudioProcessor for Processor {
    type SampleType = f32;
    /// Prepare for playback based on current audio settings
    fn s_prepare(&mut self, settings: AudioProcessorSettings) {
        self.channel_count = settings.input_channels;
        self.filters.clear();
        self.envelopes.clear();
        for _ in 0..settings.input_channels {
            let mut filter = FilterProcessor::new(FilterType::LowPass);
            filter.set_cutoff(self.filter_cutoff);
            filter.s_prepare(settings);
            self.filters.push(filter);

            let mut envelope = EnvelopeFollowerProcessor::new(
                Duration::from_secs_f32(self.envelope_attack),
                Duration::from_secs_f32(self.envelope_release),
            );
            envelope.s_prepare(settings);

            self.envelopes.push(envelope);
        }
    }

    fn s_process(&mut self, _: Self::SampleType) -> Self::SampleType {
        panic!("MultichannelProcessor must be called via s_process_frame.")
    }

    fn s_process_frame(&mut self, frame: &mut [Self::SampleType]) {
        if frame.len() != self.channel_count {
            warn!(
                "Audio frame has size {} but processor is configured for {}.",
                frame.len(),
                self.channel_count
            );
        }

        self.maybe_update_parameters();

        // Average the envelopes together.
        let mut envelope_sum = 0.0;

        // Manually call each inner processor using a slice of length 1, to
        // work around the bizarre multichannel behavior of filters.
        for chan in 0..frame.len() {
            let single_channel_frame = &mut frame[chan..chan + 1];
            self.filters[chan].s_process_frame(single_channel_frame);
            let envelope = &mut self.envelopes[chan];
            envelope.s_process_frame(single_channel_frame);
            envelope_sum += envelope.handle().state();
        }

        self.settings
            .envelope
            .set(envelope_sum / self.channel_count as f32);
    }
}
