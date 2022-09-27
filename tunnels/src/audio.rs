use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::{
    AtomicF32, AudioProcessor, AudioProcessorSettings, BufferProcessor, InterleavedAudioBuffer,
    SimpleAudioProcessor,
};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream};
use log::{info, warn};
use simple_error::{bail, simple_error};
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;

pub struct AudioInput {
    _device: Option<Device>,
    _input_stream: Option<Stream>,
    /// Handle to the atomic value written to by the audio thread.
    /// We do not return this value directly, rather we update a local version
    /// during state update to ensure that render calls see a consistent value
    /// for all fetches during the render.
    envelope_handle: Arc<AtomicF32>,
    /// Locally-stored value of the envelope.
    envelope_value: UnipolarFloat,
}

impl AudioInput {
    /// Get the names of all available input audio devices.
    pub fn devices() -> Result<Vec<String>, Box<dyn Error>> {
        let host = cpal::default_host();
        let devices = host.input_devices()?;

        let device_names = devices.map(|d| d.name().unwrap_or_else(|e| e.to_string()));
        Ok(device_names.collect())
    }

    fn offline() -> Self {
        Self {
            _device: None,
            _input_stream: None,
            envelope_handle: Arc::new(AtomicF32::new(0.0)),
            envelope_value: UnipolarFloat::ZERO,
        }
    }

    pub fn new(device_name: Option<String>) -> Result<Self, Box<dyn Error>> {
        let device_name = match device_name {
            None => {
                return Ok(Self::offline());
            }
            Some(d) => d,
        };
        let device = open_audio_input(&device_name)?;
        info!("Using audio input device {}.", device_name);
        let config: cpal::StreamConfig = device.default_input_config()?.into();

        // TODO: one envelope per channel and average them together instead of
        // just using 0th channel.
        let settings = AudioProcessorSettings {
            sample_rate: config.sample_rate.0 as f32,
            input_channels: config.channels as usize,
            output_channels: config.channels as usize, // unused
            block_size: AudioProcessorSettings::default().block_size, // unused
        };

        let mut processor = Processor::new();
        processor.s_prepare(settings);

        let envelope_handle = processor.envelope_handle();

        let mut buffer_proc = BufferProcessor(processor);

        // Need to locally buffer each frame for filtering.
        let mut audio_buf: Vec<f32> = Vec::new();

        let handle_buffer = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            audio_buf.clear();
            audio_buf.extend_from_slice(data);

            let mut interleaved_buffer =
                InterleavedAudioBuffer::new(settings.input_channels, audio_buf.as_mut_slice());
            buffer_proc.process(&mut interleaved_buffer);
        };

        let input_stream = device.build_input_stream(&config, handle_buffer, err_fn)?;

        input_stream.play()?;

        Ok(Self {
            _device: Some(device),
            _input_stream: Some(input_stream),
            envelope_handle: envelope_handle,
            envelope_value: UnipolarFloat::ZERO,
        })
    }

    /// Update the state of the locally-stored value for the envelope.
    pub fn update_state(&mut self) {
        self.envelope_value = UnipolarFloat::new(self.envelope_handle.get() as f64);
    }

    /// Return the current value of the audio envelope.
    pub fn envelope(&self) -> UnipolarFloat {
        self.envelope_value
    }
}

fn open_audio_input(name: &str) -> Result<Device, Box<dyn Error>> {
    let mut errors: Vec<String> = Vec::new();
    let host = cpal::default_host();
    for input in host.input_devices()? {
        match input.name() {
            Ok(n) if n == name => {
                return Ok(input);
            }
            Ok(_) => (),
            Err(e) => {
                errors.push(e.to_string());
            }
        }
    }
    let mut err_msg = format!("audio input {} not found", name);
    if errors.len() > 0 {
        err_msg = format!(
            "{}; some device errors occurred: {}",
            err_msg,
            errors.join(", ")
        )
    }

    bail!(err_msg);
}

struct Processor {
    envelope_handle: Arc<AtomicF32>,
    channel_count: usize,
    filters: Vec<FilterProcessor<f32>>,
    envelopes: Vec<EnvelopeFollowerProcessor>,
}

impl Processor {
    fn new() -> Self {
        Self {
            envelope_handle: Arc::new(AtomicF32::new(0.0)),
            channel_count: 0,
            filters: Vec::new(),
            envelopes: Vec::new(),
        }
    }

    fn envelope_handle(&self) -> Arc<AtomicF32> {
        self.envelope_handle.clone()
    }

    // TODO: implement settings updates somehow
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
            filter.set_cutoff(200.);
            filter.s_prepare(settings);
            self.filters.push(filter);

            let mut envelope = EnvelopeFollowerProcessor::new(
                Duration::from_millis(10),
                Duration::from_millis(100),
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

        self.envelope_handle
            .set(envelope_sum / self.channel_count as f32);
    }
}

fn err_fn(err: cpal::StreamError) {
    // TODO: if the device disconnected, or possibly if any error occurred,
    // kill the stream and start a new one.
    eprintln!("An audio input error occurred: {}", err);
}
