//! Provide an audio input stream that automatically reconnects when disconnected.
use anyhow::Result;
use anyhow::bail;
use cpal::BufferSize;
use cpal::SupportedBufferSize;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamError};
use log::{info, warn};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use crate::processor::{
    ENVELOPE_HISTORY_CAPACITY, NUM_OUTPUT_BANDS, Processor, ProcessorSettings, UpdateRate,
};
use crate::ring_buffer::{EnvelopeProducer, EnvelopeStream, envelope_ring_buffer};

pub struct ReconnectingInput {
    stop: Option<StopReconnect>,
}

impl ReconnectingInput {
    /// Create a new self-reconnecting input.
    ///
    /// The initial stream open happens on the reconnect thread (because cpal's
    /// Stream is not Send on macOS), but this method blocks until it either
    /// succeeds or fails, so the caller gets immediate error feedback.
    ///
    /// Returns `(self, envelope_streams, update_rate)` on success.
    pub fn new(
        device_name: String,
        processor_settings: ProcessorSettings,
    ) -> Result<(Self, [EnvelopeStream; NUM_OUTPUT_BANDS], UpdateRate)> {
        let (result_tx, result_rx) = channel::<Result<OpenResult>>();
        let (stop, envelope_streams, update_rate) =
            reconnect(device_name, processor_settings, result_tx, &result_rx)?;
        Ok((Self { stop: Some(stop) }, envelope_streams, update_rate))
    }
}

impl Drop for ReconnectingInput {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop()
        }
    }
}

type StopReconnect = Box<dyn FnOnce()>;

/// Try to reconnect a disconnected audio input this often.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

enum Cmd {
    Stop,
    Disconnected,
}

/// Result sent from the reconnect thread back to the caller for the initial open.
struct OpenResult {
    update_rate: UpdateRate,
    envelope_streams: [EnvelopeStream; NUM_OUTPUT_BANDS],
}

/// Spawn the reconnect thread and perform the initial stream open on it.
/// Blocks until the first open attempt completes, returning Err if it fails.
fn reconnect(
    device_name: String,
    processor_settings: ProcessorSettings,
    result_tx: Sender<Result<OpenResult>>,
    result_rx: &std::sync::mpsc::Receiver<Result<OpenResult>>,
) -> Result<(
    StopReconnect,
    [EnvelopeStream; NUM_OUTPUT_BANDS],
    UpdateRate,
)> {
    use Cmd::*;

    let (send, recv) = channel::<Cmd>();
    // Signal the thread to do the initial open.
    send.send(Disconnected).unwrap();
    let stop_sender = send.clone();
    let thread_settings = processor_settings.clone();

    let reconnect_thread = thread::spawn(move || {
        let mut _input_stream: Option<Stream> = None;
        let mut first_open = true;

        for event in recv {
            match event {
                Stop => {
                    info!("Audio reconnect thread is stopping.");
                    return;
                }
                Disconnected => {
                    // Drop the existing stream.
                    _input_stream = None;

                    let open_result = open_audio_device(&device_name).and_then(|d| {
                        build_input_stream(&d, thread_settings.clone(), send.clone())
                    });

                    match open_result {
                        Ok((stream, update_rate, envelope_streams)) => {
                            if first_open {
                                info!("Successfully opened audio input {device_name}.");
                                let _ = result_tx.send(Ok(OpenResult {
                                    update_rate,
                                    envelope_streams,
                                }));
                                first_open = false;
                            } else {
                                info!("Successfully reopened audio input {device_name}.");
                                thread_settings.set_update_rate(update_rate);
                                // Consumers from reconnect are dropped — the GUI
                                // keeps the original envelope_streams which are now abandoned.
                                // The viewer will need to re-open to get fresh ones.
                            }
                            _input_stream = Some(stream);
                        }
                        Err(e) => {
                            if first_open {
                                let _ = result_tx.send(Err(e));
                                return;
                            }
                            warn!("Unable to reopen audio input {device_name}: {e}.");
                            let sender = send.clone();
                            thread::spawn(move || {
                                thread::sleep(RECONNECT_INTERVAL);
                                sender.send(Disconnected).ok();
                            });
                        }
                    }
                }
            }
        }
    });

    // Block until the initial open attempt completes on the thread.
    let open = result_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("Audio reconnect thread exited unexpectedly"))??;

    // Write the update rate on the calling thread — no race with the show's snapshot.
    processor_settings.set_update_rate(open.update_rate);

    let stop = Box::new(move || {
        stop_sender
            .send(Cmd::Stop)
            .expect("Sending stop to reconnect thread failed");
        reconnect_thread
            .join()
            .expect("Joining reconnect thread failed");
    });
    Ok((stop, open.envelope_streams, open.update_rate))
}

fn open_audio_device(name: &str) -> Result<Device> {
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
    let mut err_msg = format!("audio input {name} not found");
    if !errors.is_empty() {
        err_msg = format!(
            "{}; some device errors occurred: {}",
            err_msg,
            errors.join(", ")
        )
    }

    bail!(err_msg);
}

fn build_input_stream(
    device: &Device,
    processor_settings: ProcessorSettings,
    disconnect_sender: Sender<Cmd>,
) -> Result<(Stream, UpdateRate, [EnvelopeStream; NUM_OUTPUT_BANDS])> {
    let supported = device.default_input_config()?;

    // Aim for about 1 ms of audio buffering latency.
    let sample_duration = 1. / supported.sample_rate().0 as f64;

    // 1000 updates/sec
    let target_latency = 1. / 1000.;

    // Compute target samples; use a power of 2, and multiply by the number of
    // channels (always gonna be 2)
    let frame_count = ((target_latency / sample_duration).round() as u32).next_power_of_two();

    // Check if this is valid for the device.
    let frame_count = match supported.buffer_size() {
        SupportedBufferSize::Unknown => {
            warn!("Unable to get supported audio device buffer sizes.");
            frame_count
        }
        SupportedBufferSize::Range { min, max } => {
            let clamped_buffer_size = frame_count.clamp(*min, *max);
            if clamped_buffer_size != frame_count {
                warn!(
                    "Target audio buffer size {frame_count} is out of range for this device; using {clamped_buffer_size}."
                );
            }
            clamped_buffer_size
        }
    };
    info!(
        "Approximate audio latency {:.1} ms.",
        frame_count as f64 * sample_duration * 1000.
    );
    let mut config: cpal::StreamConfig = supported.into();
    config.buffer_size = BufferSize::Fixed(frame_count);

    let update_rate = UpdateRate::new(config.sample_rate.0, frame_count);

    // Create envelope ring buffers — producers go to the processor, envelope_streams to the GUI.
    let mut producers = Vec::with_capacity(NUM_OUTPUT_BANDS);
    let mut envelope_streams = Vec::with_capacity(NUM_OUTPUT_BANDS);
    for _ in 0..NUM_OUTPUT_BANDS {
        let (p, c) = envelope_ring_buffer(ENVELOPE_HISTORY_CAPACITY);
        producers.push(p);
        envelope_streams.push(c);
    }
    let producers: [EnvelopeProducer; NUM_OUTPUT_BANDS] = producers
        .try_into()
        .ok()
        .expect("correct number of producers");
    let envelope_streams: [EnvelopeStream; NUM_OUTPUT_BANDS] = envelope_streams
        .try_into()
        .ok()
        .expect("correct number of envelope_streams");

    let mut processor = Processor::new(
        processor_settings,
        config.sample_rate.0,
        config.channels as usize,
        producers,
    );

    let handle_buffer = move |interleaved_buffer: &[f32], _: &cpal::InputCallbackInfo| {
        processor.process(interleaved_buffer);
    };

    let handle_error = move |err: StreamError| match err {
        StreamError::BackendSpecific { err } => {
            eprintln!("An audio input error occurred: {err}");
        }
        StreamError::DeviceNotAvailable => {
            disconnect_sender.send(Cmd::Disconnected).ok();
            warn!("Audio input disconnected.");
        }
    };

    let input_stream = device.build_input_stream(&config, handle_buffer, handle_error, None)?;

    input_stream.play()?;
    Ok((input_stream, update_rate, envelope_streams))
}
