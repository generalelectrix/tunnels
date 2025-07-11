//! Provide an audio input stream that automatically reconnects when disconnected.
use anyhow::bail;
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::BufferSize;
use cpal::SupportedBufferSize;
use cpal::{Device, Stream, StreamError};
use log::{info, warn};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use super::processor::{Processor, ProcessorSettings};

pub struct ReconnectingInput {
    stop: Option<StopReconnect>,
}

impl ReconnectingInput {
    /// Create a new self-reconnecting input.
    /// Device disconnection is handled asynchronously and will attempt to
    /// reconnect the device until this struct is dropped.
    pub fn new(device_name: String, processor_settings: ProcessorSettings) -> Self {
        Self {
            stop: Some(reconnect(device_name, processor_settings)),
        }
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

/// Spawn a thread to handle device disconnection.
/// Return a closure that can be called to terminate the input stream, blocking
/// until it completes.
fn reconnect(device_name: String, processor_settings: ProcessorSettings) -> StopReconnect {
    enum Cmd {
        Stop,
        Disconnected,
    }
    use Cmd::*;

    let (send, recv) = channel::<Cmd>();
    // Load an initial command into the queue to open input.
    send.send(Cmd::Disconnected).unwrap();
    let disconnected_sender = send.clone();

    let reconnect_thread = thread::spawn(move || {
        let mut _input_stream = None;
        for event in recv {
            match event {
                Stop => {
                    info!("Audio reconnect thread is stopping.");
                    return;
                }
                Disconnected => {
                    // Drop the existing stream.
                    {
                        _input_stream = None;
                    }
                    // Try to re-open.
                    let sender = disconnected_sender.clone();
                    let reopen_result =
                        create_input_stream(&device_name, processor_settings.clone(), move || {
                            sender.send(Disconnected).ok();
                            warn!("Audio input disconnected.");
                        });

                    match reopen_result {
                        Ok(input) => {
                            info!("Successfully opened audio input {device_name}.");
                            _input_stream = Some(input);
                        }
                        Err(e) => {
                            warn!("Unable to reopen audio input {device_name}: {e}.");
                            let sender = disconnected_sender.clone();
                            // Spawn a thread to wake us up and try again after a delay.
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

    Box::new(move || {
        send.send(Stop)
            .expect("Sending stop to reconnect thread failed");
        reconnect_thread
            .join()
            .expect("Joining reconnect thread failed");
    })
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

fn create_input_stream<F>(
    device_name: &str,
    processor_settings: ProcessorSettings,
    mut on_disconnect: F,
) -> Result<Stream>
where
    F: FnMut() + Send + 'static,
{
    let device = open_audio_device(device_name)?;
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
                warn!("Target audio buffer size {frame_count} is out of range for this device; using {clamped_buffer_size}.");
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

    let mut processor = Processor::new(
        processor_settings,
        config.sample_rate.0,
        config.channels as usize,
    );

    let handle_buffer = move |interleaved_buffer: &[f32], _: &cpal::InputCallbackInfo| {
        processor.process(interleaved_buffer);
    };

    let handle_error = move |err: StreamError| match err {
        StreamError::BackendSpecific { err } => {
            eprintln!("An audio input error occurred: {err}");
        }
        StreamError::DeviceNotAvailable => {
            on_disconnect();
        }
    };

    let input_stream = device.build_input_stream(&config, handle_buffer, handle_error, None)?;

    input_stream.play()?;
    Ok(input_stream)
}
