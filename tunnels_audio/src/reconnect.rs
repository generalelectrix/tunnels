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

use crate::processor::{Processor, ProcessorSettings};

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
    /// The provided `device` is used for the initial open, avoiding device
    /// re-enumeration. On reconnection after a disconnect, the device is
    /// looked up by name.
    pub fn new(
        device_name: String,
        device: Device,
        processor_settings: ProcessorSettings,
    ) -> Result<Self> {
        let (result_tx, result_rx) = channel::<Result<()>>();
        Ok(Self {
            stop: Some(reconnect(
                device_name,
                device,
                processor_settings,
                result_tx,
                &result_rx,
            )?),
        })
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

/// Spawn the reconnect thread and perform the initial stream open on it.
/// Blocks until the first open attempt completes, returning Err if it fails.
fn reconnect(
    device_name: String,
    device: Device,
    processor_settings: ProcessorSettings,
    result_tx: Sender<Result<()>>,
    result_rx: &std::sync::mpsc::Receiver<Result<()>>,
) -> Result<StopReconnect> {
    use Cmd::*;

    let (send, recv) = channel::<Cmd>();
    // Signal the thread to do the initial open.
    send.send(Disconnected).unwrap();
    let stop_sender = send.clone();

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

                    // For the initial open, use the device handle directly (no
                    // enumeration). For reconnection, look up by name.
                    let device_result = if first_open {
                        Ok(device.clone())
                    } else {
                        find_device_by_name(&device_name)
                    };

                    let open_result = device_result.and_then(|d| {
                        build_input_stream(&d, processor_settings.clone(), send.clone())
                    });

                    match open_result {
                        Ok(input) => {
                            if first_open {
                                info!("Successfully opened audio input {device_name}.");
                                let _ = result_tx.send(Ok(()));
                                first_open = false;
                            } else {
                                info!("Successfully reopened audio input {device_name}.");
                            }
                            _input_stream = Some(input);
                        }
                        Err(e) => {
                            if first_open {
                                // Report the error to the caller and exit.
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
    let initial_result = result_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("Audio reconnect thread exited unexpectedly"))?;
    initial_result?;

    Ok(Box::new(move || {
        stop_sender
            .send(Cmd::Stop)
            .expect("Sending stop to reconnect thread failed");
        reconnect_thread
            .join()
            .expect("Joining reconnect thread failed");
    }))
}

/// Find a device by name. Used only for reconnection after a disconnect.
fn find_device_by_name(name: &str) -> Result<Device> {
    let host = cpal::default_host();
    for device in host.devices()? {
        match device.name() {
            Ok(n) if n == name => return Ok(device),
            Ok(_) => (),
            Err(_) => (),
        }
    }
    bail!("audio device {name} not found");
}

fn build_input_stream(
    device: &Device,
    processor_settings: ProcessorSettings,
    disconnect_sender: Sender<Cmd>,
) -> Result<Stream> {
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

    // Set the update rate once — this is the rate at which the audio callback
    // fires and envelope ring buffers are pushed.
    let update_rate = config.sample_rate.0 as f32 / frame_count as f32;
    processor_settings.update_rate.set(update_rate);

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
            disconnect_sender.send(Cmd::Disconnected).ok();
            warn!("Audio input disconnected.");
        }
    };

    let input_stream = device.build_input_stream(&config, handle_buffer, handle_error, None)?;

    input_stream.play()?;
    Ok(input_stream)
}
