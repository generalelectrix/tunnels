//! A single-slot handoff of the latest [`AudioFrame`] from one thread to
//! another: the writer never waits, the reader never waits, and a read always
//! sees one whole frame.

use tunnels_lib::audio::AudioFrame;

/// Create a connected producer and reader, the reader seeing an all-zero
/// frame until the first is published.
pub fn frame_buffer() -> (FrameProducer, FrameReader) {
    let (input, output) = triple_buffer::triple_buffer(&AudioFrame::default());
    (FrameProducer(input), FrameReader(output))
}

/// The writing end of a frame buffer.
pub struct FrameProducer(triple_buffer::Input<AudioFrame>);

impl FrameProducer {
    /// Make a frame the latest, replacing any the reader has not read.
    pub fn publish(&mut self, frame: AudioFrame) {
        self.0.write(frame);
    }
}

/// The reading end of a frame buffer.
pub struct FrameReader(triple_buffer::Output<AudioFrame>);

impl FrameReader {
    /// The latest published frame.
    pub fn latest(&mut self) -> AudioFrame {
        *self.0.read()
    }
}
