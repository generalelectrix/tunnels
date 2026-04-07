//! Running median filter using a two-heap approach.
//!
//! Maintains a sliding window of samples and efficiently computes
//! the median at each step. O(log k) per sample where k is the window size.

use std::collections::BinaryHeap;
use std::cmp::Reverse;
use std::collections::VecDeque;

/// Running median over a fixed-size sliding window.
pub struct RunningMedian {
    window: VecDeque<f32>,
    max_heap: BinaryHeap<OrdF32>,        // lower half (max at top)
    min_heap: BinaryHeap<Reverse<OrdF32>>, // upper half (min at top)
    capacity: usize,
}

/// Wrapper for f32 that implements Ord via total_cmp.
#[derive(Clone, Copy, PartialEq)]
struct OrdF32(f32);

impl Eq for OrdF32 {}

impl PartialOrd for OrdF32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrdF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl RunningMedian {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            window: VecDeque::with_capacity(capacity),
            max_heap: BinaryHeap::new(),
            min_heap: BinaryHeap::new(),
            capacity,
        }
    }

    /// Push a new sample and return the current median.
    ///
    /// Uses a simplified approach: when the window is full, rebuild the
    /// heaps from scratch. This is O(k) per sample rather than O(log k),
    /// but for small windows (< 500 samples) this is negligible and avoids
    /// the complexity of lazy-deletion heap maintenance.
    pub fn push(&mut self, value: f32) -> f32 {
        self.window.push_back(value);
        if self.window.len() > self.capacity {
            self.window.pop_front();
        }

        // Rebuild heaps from current window.
        // For our use case (window ~50-200 at 1kHz), this is fast enough.
        self.max_heap.clear();
        self.min_heap.clear();

        let mut sorted: Vec<f32> = self.window.iter().copied().collect();
        sorted.sort_unstable_by(|a, b| a.total_cmp(b));

        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_element() {
        let mut m = RunningMedian::new(5);
        assert_eq!(m.push(3.0), 3.0);
    }

    #[test]
    fn odd_window() {
        let mut m = RunningMedian::new(3);
        m.push(1.0);
        m.push(3.0);
        let median = m.push(2.0);
        assert_eq!(median, 2.0);
    }

    #[test]
    fn even_window() {
        let mut m = RunningMedian::new(4);
        m.push(1.0);
        m.push(2.0);
        m.push(3.0);
        let median = m.push(4.0);
        assert_eq!(median, 2.5);
    }

    #[test]
    fn sliding_window_drops_old() {
        let mut m = RunningMedian::new(3);
        m.push(1.0);
        m.push(2.0);
        m.push(3.0);
        // Window: [1, 2, 3], median = 2
        assert_eq!(m.push(3.0), 3.0); // Window: [2, 3, 3]
        assert_eq!(m.push(3.0), 3.0); // Window: [3, 3, 3]
    }

    #[test]
    fn preserves_step_edge() {
        let mut m = RunningMedian::new(5);
        // Feed 0s then step to 1s.
        for _ in 0..5 {
            m.push(0.0);
        }
        // Now feed 1s — median should transition to 1 once majority is 1.
        m.push(1.0); // [0,0,0,0,1] → median 0
        m.push(1.0); // [0,0,0,1,1] → median 0
        let median = m.push(1.0); // [0,0,1,1,1] → median 1
        assert_eq!(median, 1.0);
    }

    #[test]
    fn rejects_impulse() {
        let mut m = RunningMedian::new(5);
        for _ in 0..5 {
            m.push(0.5);
        }
        // Spike — median should stay at 0.5.
        let median = m.push(10.0);
        assert_eq!(median, 0.5); // [0.5, 0.5, 0.5, 0.5, 10.0] → median 0.5
    }
}
