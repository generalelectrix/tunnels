//! Lock-free single-producer single-consumer ring buffer for streaming
//! time-series data from the audio thread to the GUI thread.
//!
//! The producer (audio thread) pushes one sample per buffer callback (~1kHz).
//! The consumer (GUI thread) reads all available samples each frame (~60Hz).
//! When the buffer is full, new samples overwrite the oldest (lossy).

use audio_processor_traits::AtomicF32;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SignalRingBuffer {
    buffer: Box<[AtomicF32]>,
    /// Monotonically increasing write position. Wraps via bitmask.
    write_pos: AtomicUsize,
    /// Capacity must be a power of two.
    capacity: usize,
}

impl SignalRingBuffer {
    /// Create a new ring buffer with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is not a power of two or is zero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two() && capacity > 0);
        let buffer: Vec<AtomicF32> = (0..capacity).map(|_| AtomicF32::new(0.0)).collect();
        Self {
            buffer: buffer.into_boxed_slice(),
            write_pos: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Push a sample. Called from the audio thread.
    /// Lock-free, wait-free, single-producer only.
    pub fn push(&self, value: f32) {
        let pos = self.write_pos.load(Ordering::Relaxed);
        let index = pos & (self.capacity - 1);
        self.buffer[index].store(value, Ordering::Relaxed);
        // Release ensures the store above is visible before the position update.
        self.write_pos.store(pos.wrapping_add(1), Ordering::Release);
    }

    /// Read all samples written since the last read.
    /// Returns them in chronological order.
    /// Called from the GUI thread (single consumer).
    ///
    /// `last_read_pos` is caller-owned state tracking where we left off.
    pub fn drain_into(&self, dest: &mut Vec<f32>, last_read_pos: &mut usize) {
        // Acquire ensures we see all stores that happened before the position update.
        let current_write = self.write_pos.load(Ordering::Acquire);

        let available = current_write.wrapping_sub(*last_read_pos);

        if available == 0 {
            return;
        }

        // If we've fallen behind by more than the buffer capacity,
        // skip to the oldest available data.
        let (start, count) = if available > self.capacity {
            (current_write - self.capacity, self.capacity)
        } else {
            (*last_read_pos, available)
        };

        dest.reserve(count);
        for i in 0..count {
            let index = start.wrapping_add(i) & (self.capacity - 1);
            // Relaxed is fine here — the Acquire on write_pos already
            // established the happens-before relationship.
            dest.push(self.buffer[index].get());
        }

        *last_read_pos = current_write;
    }

    /// Return the current write position (for initializing a consumer's last_read_pos).
    pub fn write_pos(&self) -> usize {
        self.write_pos.load(Ordering::Acquire)
    }

    /// Return the capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

// Safety: SignalRingBuffer is Send+Sync because all fields are atomic.
// The buffer is a Box<[AtomicF32]> which is Send+Sync, and the
// write_pos is AtomicUsize which is Send+Sync.
unsafe impl Send for SignalRingBuffer {}
unsafe impl Sync for SignalRingBuffer {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_drain_basic() {
        let rb = SignalRingBuffer::new(8);
        let mut last = rb.write_pos();
        let mut out = Vec::new();

        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);

        rb.drain_into(&mut out, &mut last);
        assert_eq!(out, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn drain_empty() {
        let rb = SignalRingBuffer::new(8);
        let mut last = rb.write_pos();
        let mut out = Vec::new();

        rb.drain_into(&mut out, &mut last);
        assert!(out.is_empty());
    }

    #[test]
    fn drain_incremental() {
        let rb = SignalRingBuffer::new(8);
        let mut last = rb.write_pos();
        let mut out = Vec::new();

        rb.push(1.0);
        rb.push(2.0);
        rb.drain_into(&mut out, &mut last);
        assert_eq!(out, vec![1.0, 2.0]);

        out.clear();
        rb.push(3.0);
        rb.drain_into(&mut out, &mut last);
        assert_eq!(out, vec![3.0]);
    }

    #[test]
    fn overflow_drops_oldest() {
        let rb = SignalRingBuffer::new(4);
        let mut last = rb.write_pos();
        let mut out = Vec::new();

        // Write 6 samples into a 4-slot buffer.
        for i in 0..6 {
            rb.push(i as f32);
        }

        rb.drain_into(&mut out, &mut last);
        // Should get the last 4 samples.
        assert_eq!(out, vec![2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn write_pos_initialized_skips_history() {
        let rb = SignalRingBuffer::new(8);

        rb.push(1.0);
        rb.push(2.0);

        // Initialize last_read_pos now — should not see the previous 2 samples.
        let mut last = rb.write_pos();
        let mut out = Vec::new();

        rb.push(3.0);
        rb.drain_into(&mut out, &mut last);
        assert_eq!(out, vec![3.0]);
    }

    #[test]
    #[should_panic]
    fn non_power_of_two_panics() {
        SignalRingBuffer::new(5);
    }

    #[test]
    #[should_panic]
    fn zero_capacity_panics() {
        SignalRingBuffer::new(0);
    }
}
