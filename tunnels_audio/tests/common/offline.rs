//! Drives a processor over a rendered signal the way a device would:
//! fixed-size buffers, ring buffers drained after each.

use tunnels_audio::processor::{
    NUM_OUTPUT_BANDS, Processor, ProcessorSettings, envelope_ring_buffers,
};

/// Run a stereo signal through a fresh processor in buffers of
/// `frames_per_buffer` frames, calling `per_buffer` after each with the
/// buffer index, the processor, and the eight normalized band outputs.
pub fn run_stereo(
    sample_rate: u32,
    frames_per_buffer: usize,
    settings: ProcessorSettings,
    signal: &[[f32; 2]],
    mut per_buffer: impl FnMut(usize, &Processor, &[f32; NUM_OUTPUT_BANDS]),
) {
    let buffers = envelope_ring_buffers();
    let mut streams = buffers.streams;
    let mut processor = Processor::new(settings, sample_rate, 2, buffers.producers);
    let mut interleaved = Vec::with_capacity(frames_per_buffer * 2);
    let mut drained = Vec::new();
    for (i, chunk) in signal.chunks(frames_per_buffer).enumerate() {
        interleaved.clear();
        for frame in chunk {
            interleaved.extend_from_slice(frame);
        }
        processor.process(&interleaved);
        let mut bands = [0.0; NUM_OUTPUT_BANDS];
        for (band, stream) in bands.iter_mut().zip(&mut streams) {
            drained.clear();
            stream.drain_into(&mut drained);
            *band = *drained.last().expect("one value per buffer");
        }
        per_buffer(i, &processor, &bands);
    }
}
