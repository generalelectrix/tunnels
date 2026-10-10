//! Drives a processor over a rendered signal the way a device would:
//! fixed-size buffers, the frame read and the ring buffers drained after each.

use std::num::NonZeroUsize;
use tunnels_audio::AudioFrame;
use tunnels_audio::frame_buffer::frame_buffer;
use tunnels_audio::processor::{
    NormalizerTuning, Processor, ProcessorSettings, envelope_ring_buffers,
};

/// Run a stereo signal through a fresh processor in buffers of
/// `frames_per_buffer` frames, calling `per_buffer` after each with the
/// buffer index, the processor, and the frame it published. Panics unless
/// each role's ring buffer received exactly the frame's value.
pub fn run_stereo(
    sample_rate: u32,
    frames_per_buffer: usize,
    settings: ProcessorSettings,
    signal: &[[f32; 2]],
    per_buffer: impl FnMut(usize, &Processor, &AudioFrame),
) {
    run_stereo_tuned(
        sample_rate,
        frames_per_buffer,
        settings,
        NormalizerTuning::DEFAULT,
        signal,
        per_buffer,
    );
}

/// `run_stereo` with the processor's normalizer tuning replaced.
pub fn run_stereo_tuned(
    sample_rate: u32,
    frames_per_buffer: usize,
    settings: ProcessorSettings,
    tuning: NormalizerTuning,
    signal: &[[f32; 2]],
    mut per_buffer: impl FnMut(usize, &Processor, &AudioFrame),
) {
    let buffers = envelope_ring_buffers();
    let mut streams = buffers.streams;
    let (producer, mut frames) = frame_buffer();
    let mut processor = Processor::new(
        settings,
        sample_rate,
        NonZeroUsize::new(2).expect("two channels"),
        buffers.producers,
        producer,
    );
    processor.set_normalizer_tuning(tuning);
    let mut interleaved = Vec::with_capacity(frames_per_buffer * 2);
    let mut drained = Vec::new();
    for (i, chunk) in signal.chunks(frames_per_buffer).enumerate() {
        interleaved.clear();
        for frame in chunk {
            interleaved.extend_from_slice(frame);
        }
        processor.process(&interleaved);
        let frame = frames.latest();
        for (role, stream) in frame.roles().iter().zip(&mut streams) {
            drained.clear();
            stream.drain_into(&mut drained);
            assert_eq!(drained, [*role], "one value per buffer, the frame's");
        }
        per_buffer(i, &processor, &frame);
    }
}
