use crate::master_ui::EmitStateChange as EmitShowStateChange;
use crate::transient_indicator::TransientIndicator;
use audio_processor_analysis::envelope_follower_processor::EnvelopeFollowerProcessor;
use audio_processor_traits::{
    AtomicF32, AudioProcessor, AudioProcessorSettings, BufferProcessor, InterleavedAudioBuffer,
    SimpleAudioProcessor,
};
use augmented_dsp_filters::rbj::{FilterProcessor, FilterType};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream};
use log::{info, warn};
use simple_error::bail;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tunnels_lib::number::UnipolarFloat;

pub struct AudioInput {
    _device: Option<Device>,
    _input_stream: Option<Stream>,
    processor_handle: Arc<ProcessorHandle>,
    /// Locally-stored value of the envelope.
    envelope_value: UnipolarFloat,
    /// Should we send monitor updates?
    monitor: bool,
    /// Envelope gain factor.
    gain: f64,
    /// Transient envelope clip indicator.
    clip_indicator: TransientIndicator,
}

impl AudioInput {
    const CLIP_INDICATOR_DURATION: Duration = Duration::from_millis(100);
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
            processor_handle: Arc::new(ProcessorHandle::default()),
            envelope_value: UnipolarFloat::ZERO,
            monitor: false,
            gain: 1.0,
            clip_indicator: TransientIndicator::new(Self::CLIP_INDICATOR_DURATION),
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

        let settings = AudioProcessorSettings {
            sample_rate: config.sample_rate.0 as f32,
            input_channels: config.channels as usize,
            output_channels: config.channels as usize, // unused
            block_size: AudioProcessorSettings::default().block_size, // unused
        };

        let mut processor = Processor::new();
        processor.s_prepare(settings);

        let handle = processor.handle();

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
            processor_handle: handle,
            envelope_value: UnipolarFloat::ZERO,
            monitor: false,
            gain: 1.0,
            clip_indicator: TransientIndicator::new(Self::CLIP_INDICATOR_DURATION),
        })
    }

    /// Update the state of audio control.
    /// The audio control system may need to emit state update.
    pub fn update_state<E: EmitStateChange>(&mut self, delta_t: Duration, emitter: &mut E) {
        let raw_envelope = self.processor_handle.envelope.get() as f64;
        let scaled_envelope = raw_envelope * self.gain;
        let clipping = scaled_envelope > 1.0;
        self.envelope_value = UnipolarFloat::new(scaled_envelope);
        if self.monitor {
            emitter.emit_audio_state_change(StateChange::EnvelopeValue(self.envelope_value));
            if let Some(clip_state) = self.clip_indicator.update_state(delta_t, clipping) {
                emitter.emit_audio_state_change(StateChange::IsClipping(clip_state));
            }
        }
    }

    /// Emit the current value of all controllable state.
    pub fn emit_state<E: EmitStateChange>(&self, emitter: &mut E) {
        use StateChange::*;
        emitter.emit_audio_state_change(EnvelopeValue(self.envelope_value));
        emitter.emit_audio_state_change(Monitor(self.monitor));
        emitter.emit_audio_state_change(FilterCutoff(self.processor_handle.filter_cutoff.get()));
        emitter.emit_audio_state_change(EnvelopeAttack(Duration::from_secs_f32(
            self.processor_handle.envelope_attack.get(),
        )));
        emitter.emit_audio_state_change(EnvelopeRelease(Duration::from_secs_f32(
            self.processor_handle.envelope_release.get(),
        )));
        emitter.emit_audio_state_change(Gain(self.gain));
        emitter.emit_audio_state_change(IsClipping(self.clip_indicator.state()));
    }

    /// Handle a control event.
    /// Emit any state changes that have happened as a result of handling.
    pub fn control<E: EmitStateChange>(&mut self, msg: ControlMessage, emitter: &mut E) {
        use ControlMessage::*;
        match msg {
            ToggleMonitor => {
                self.monitor = !self.monitor;
                emitter.emit_audio_state_change(StateChange::Monitor(self.monitor));
                if !self.monitor {
                    emitter
                        .emit_audio_state_change(StateChange::EnvelopeValue(UnipolarFloat::ZERO));
                    emitter.emit_audio_state_change(StateChange::IsClipping(false));
                    self.clip_indicator.reset();
                }
            }
            ResetParameters => {
                self.processor_handle.reset_defaults();
                self.gain = 1.0;
                self.clip_indicator.reset();
                self.emit_state(emitter);
            }
            Set(sc) => self.handle_state_change(sc, emitter),
        }
    }

    fn handle_state_change<E: EmitStateChange>(&mut self, sc: StateChange, emitter: &mut E) {
        use StateChange::*;
        match sc {
            EnvelopeValue(_) => (), // output only, input ignored
            Monitor(v) => self.monitor = v,
            FilterCutoff(v) => {
                if v <= 0. {
                    warn!("Invalid filter cutoff frequency {} (<= 0).", v);
                    return;
                }
                self.processor_handle.filter_cutoff.set(v);
            }
            EnvelopeAttack(v) => self.processor_handle.envelope_attack.set(v.as_secs_f32()),
            EnvelopeRelease(v) => self.processor_handle.envelope_release.set(v.as_secs_f32()),
            Gain(v) => {
                if v < 0. {
                    warn!("Invalid audio envelope gain {} (< 0).", v);
                    return;
                }
                info!("Gain: {}", v);
                self.gain = v;
            }
            IsClipping(_) => {
                return; // output only
            }
        };
        emitter.emit_audio_state_change(sc);
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

#[derive(Clone)]
struct ProcessorHandle {
    envelope: AtomicF32,
    filter_cutoff: AtomicF32,    // Hz
    envelope_attack: AtomicF32,  // sec
    envelope_release: AtomicF32, // sec
}

impl ProcessorHandle {
    const DEFAULT_FILTER_CUTOFF: f32 = 200.;
    const DEFAULT_ENVELOPE_ATTACK: f32 = 0.01;
    const DEFAULT_ENVELOPE_RELEASE: f32 = 0.1;

    fn reset_defaults(&self) {
        self.filter_cutoff.set(Self::DEFAULT_FILTER_CUTOFF);
        self.envelope_attack.set(Self::DEFAULT_ENVELOPE_ATTACK);
        self.envelope_release.set(Self::DEFAULT_ENVELOPE_RELEASE);
    }
}

impl Default for ProcessorHandle {
    fn default() -> Self {
        Self {
            envelope: AtomicF32::new(0.0),
            filter_cutoff: AtomicF32::new(Self::DEFAULT_FILTER_CUTOFF),
            envelope_attack: AtomicF32::new(Self::DEFAULT_ENVELOPE_ATTACK),
            envelope_release: AtomicF32::new(Self::DEFAULT_ENVELOPE_RELEASE),
        }
    }
}

struct Processor {
    handle: Arc<ProcessorHandle>,
    filter_cutoff: f32,
    envelope_attack: f32,
    envelope_release: f32,
    channel_count: usize,
    filters: Vec<FilterProcessor<f32>>,
    envelopes: Vec<EnvelopeFollowerProcessor>,
}

impl Processor {
    fn new() -> Self {
        let handle = Arc::new(ProcessorHandle::default());
        Self {
            filter_cutoff: handle.filter_cutoff.get(),
            envelope_attack: handle.envelope_attack.get(),
            envelope_release: handle.envelope_release.get(),
            handle,
            channel_count: 0,
            filters: Vec::new(),
            envelopes: Vec::new(),
        }
    }

    fn handle(&self) -> Arc<ProcessorHandle> {
        self.handle.clone()
    }

    /// Load current parameters and update filters/envelopes if they have changed.
    fn maybe_update_parameters(&mut self) {
        let new_filter_cutoff = self.handle.filter_cutoff.get();
        if new_filter_cutoff != self.filter_cutoff {
            info!("Updating filter cutoff to {}", new_filter_cutoff);
            self.filter_cutoff = new_filter_cutoff;
            for filter in self.filters.iter_mut() {
                filter.set_cutoff(new_filter_cutoff);
            }
        }
        let new_envelope_attack = self.handle.envelope_attack.get();
        let new_envelope_release = self.handle.envelope_release.get();
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

        self.handle
            .envelope
            .set(envelope_sum / self.channel_count as f32);
    }
}

fn err_fn(err: cpal::StreamError) {
    // TODO: if the device disconnected, or possibly if any error occurred,
    // kill the stream and start a new one.
    eprintln!("An audio input error occurred: {}", err);
}

#[derive(Debug)]
pub enum StateChange {
    Monitor(bool),
    EnvelopeValue(UnipolarFloat),
    FilterCutoff(f32),
    EnvelopeAttack(Duration),
    EnvelopeRelease(Duration),
    Gain(f64),
    IsClipping(bool),
}

pub enum ControlMessage {
    Set(StateChange),
    ToggleMonitor,
    ResetParameters,
}

pub trait EmitStateChange {
    fn emit_audio_state_change(&mut self, sc: StateChange);
}

impl<T: EmitShowStateChange> EmitStateChange for T {
    fn emit_audio_state_change(&mut self, sc: StateChange) {
        use crate::show::StateChange as ShowStateChange;
        self.emit(ShowStateChange::Audio(sc))
    }
}
