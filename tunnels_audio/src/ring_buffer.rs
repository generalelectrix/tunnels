//! Lock-free single-producer single-consumer ring buffer for streaming
//! envelope values from the audio thread to the GUI thread.
//!
//! Thin wrappers over `rtrb` that expose only the operations we need
//! and keep the dependency from leaking into the rest of the codebase.

use tunnels_lib::audio::UnipolarF32;

/// Create a producer/consumer pair backed by a ring buffer of the given capacity.
pub fn envelope_ring_buffer(capacity: usize) -> (EnvelopeProducer, EnvelopeStream) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    (EnvelopeProducer(producer), EnvelopeStream(consumer))
}

/// Producer side of an envelope ring buffer. Lives on the audio thread.
pub struct EnvelopeProducer(rtrb::Producer<UnipolarF32>);

impl EnvelopeProducer {
    /// Push a sample. If the buffer is full, the sample is silently dropped.
    pub fn push(&mut self, value: UnipolarF32) {
        let _ = self.0.push(value);
    }
}

/// Consumer side of an envelope ring buffer. Lives on the GUI thread.
pub struct EnvelopeStream(rtrb::Consumer<UnipolarF32>);

impl EnvelopeStream {
    /// Read all available samples into `dest`.
    pub fn drain_into(&mut self, dest: &mut Vec<UnipolarF32>) {
        while let Ok(value) = self.0.pop() {
            dest.push(value);
        }
    }

    /// Discard all pending data without reading it.
    pub fn clear(&mut self) {
        let available = self.0.slots();
        if available > 0
            && let Ok(chunk) = self.0.read_chunk(available)
        {
            chunk.commit_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A distinct envelope value per small integer.
    fn u(i: usize) -> UnipolarF32 {
        UnipolarF32::new(i as f32 / 10.0)
    }

    #[test]
    fn push_and_drain_basic() {
        let (mut p, mut c) = envelope_ring_buffer(8);
        let mut out = Vec::new();

        p.push(u(1));
        p.push(u(2));
        p.push(u(3));

        c.drain_into(&mut out);
        assert_eq!(out, vec![u(1), u(2), u(3)]);
    }

    #[test]
    fn drain_empty() {
        let (_p, mut c) = envelope_ring_buffer(8);
        let mut out = Vec::new();

        c.drain_into(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn drain_incremental() {
        let (mut p, mut c) = envelope_ring_buffer(8);
        let mut out = Vec::new();

        p.push(u(1));
        p.push(u(2));
        c.drain_into(&mut out);
        assert_eq!(out, vec![u(1), u(2)]);

        out.clear();
        p.push(u(3));
        c.drain_into(&mut out);
        assert_eq!(out, vec![u(3)]);
    }

    #[test]
    fn full_buffer_drops_new_samples() {
        let (mut p, mut c) = envelope_ring_buffer(4);
        let mut out = Vec::new();

        // Write 6 samples into a 4-slot buffer — last 2 are dropped.
        for i in 0..6 {
            p.push(u(i));
        }

        c.drain_into(&mut out);
        // rtrb is non-lossy: the first 4 are kept, the last 2 are dropped.
        assert_eq!(out, vec![u(0), u(1), u(2), u(3)]);
    }

    #[test]
    fn clear_discards_data() {
        let (mut p, mut c) = envelope_ring_buffer(8);

        p.push(u(1));
        p.push(u(2));
        p.push(u(3));

        c.clear();

        let mut out = Vec::new();
        c.drain_into(&mut out);
        assert!(out.is_empty());
    }
}
