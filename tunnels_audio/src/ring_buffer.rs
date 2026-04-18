//! Lock-free single-producer single-consumer ring buffer for streaming
//! f32 samples from the audio thread to the GUI thread.
//!
//! Thin wrappers over `rtrb` that expose only the operations we need
//! and keep the dependency from leaking into the rest of the codebase.

/// Create a producer/consumer pair backed by a ring buffer of the given capacity.
pub fn envelope_ring_buffer(capacity: usize) -> (EnvelopeProducer, EnvelopeStream) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    (EnvelopeProducer(producer), EnvelopeStream(consumer))
}

/// Producer side of an envelope ring buffer. Lives on the audio thread.
pub struct EnvelopeProducer(rtrb::Producer<f32>);

impl EnvelopeProducer {
    /// Push a sample. If the buffer is full, the sample is silently dropped.
    pub fn push(&mut self, value: f32) {
        let _ = self.0.push(value);
    }
}

/// Consumer side of an envelope ring buffer. Lives on the GUI thread.
pub struct EnvelopeStream(rtrb::Consumer<f32>);

impl EnvelopeStream {
    /// Read all available samples into `dest`.
    pub fn drain_into(&mut self, dest: &mut Vec<f32>) {
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

    #[test]
    fn push_and_drain_basic() {
        let (mut p, mut c) = envelope_ring_buffer(8);
        let mut out = Vec::new();

        p.push(1.0);
        p.push(2.0);
        p.push(3.0);

        c.drain_into(&mut out);
        assert_eq!(out, vec![1.0, 2.0, 3.0]);
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

        p.push(1.0);
        p.push(2.0);
        c.drain_into(&mut out);
        assert_eq!(out, vec![1.0, 2.0]);

        out.clear();
        p.push(3.0);
        c.drain_into(&mut out);
        assert_eq!(out, vec![3.0]);
    }

    #[test]
    fn full_buffer_drops_new_samples() {
        let (mut p, mut c) = envelope_ring_buffer(4);
        let mut out = Vec::new();

        // Write 6 samples into a 4-slot buffer — last 2 are dropped.
        for i in 0..6 {
            p.push(i as f32);
        }

        c.drain_into(&mut out);
        // rtrb is non-lossy: the first 4 are kept, the last 2 are dropped.
        assert_eq!(out, vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn clear_discards_data() {
        let (mut p, mut c) = envelope_ring_buffer(8);

        p.push(1.0);
        p.push(2.0);
        p.push(3.0);

        c.clear();

        let mut out = Vec::new();
        c.drain_into(&mut out);
        assert!(out.is_empty());
    }
}
