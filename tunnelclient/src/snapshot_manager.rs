//! Handle emptying a queue of snapshots, maintaining a time-ordered collection,
//! and interpolating between them on demand.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tunnels_lib::Timestamp;
use tunnels_lib::{LayerCollection, Snapshot};

pub type SnapshotManagerHandle = Arc<Mutex<SnapshotManager>>;

/// Handle receiving and maintaining a collection of snapshots.
/// Provide interpolated snapshots on request.
#[derive(Default)]
pub struct SnapshotManager {
    snapshots: VecDeque<Snapshot>, // Ordered queue of snapshots; latest is snapshots.front()
    oldest_relevant_snapshot_time: Timestamp,
}

pub enum SnapshotFetchResult {
    NoData,                        // no data is available at all
    Good(LayerCollection),         // Both snapshots were available.
    MissingNewer(LayerCollection), // Data is out-of-date for current timestamp.
    MissingOlder(LayerCollection), // We only have snapshot data newer than requested.
    Error(Vec<Snapshot>),          // Something went wrong and we couldn't perform interpolation.
}

enum InsertStrategy {
    PushFront,
    Insert,
}

impl SnapshotManager {
    /// Add a new snapshot, ensuring the collection remains ordered.
    pub fn insert_snapshot(&mut self, snapshot: Snapshot) {
        let insert_strategy = match self.snapshots.front() {
            None => InsertStrategy::PushFront,
            Some(s) => {
                if snapshot.time > s.time {
                    InsertStrategy::PushFront
                } else {
                    InsertStrategy::Insert
                }
            }
        };
        match insert_strategy {
            InsertStrategy::PushFront => {
                self.snapshots.push_front(snapshot);
            }
            InsertStrategy::Insert => {
                let mut insert_index = 0;
                // iterate backwards and find the right spot to insert
                for (index, older_snapshot) in self.snapshots.iter().enumerate() {
                    if snapshot.time > older_snapshot.time {
                        insert_index = index;
                        break;
                    }
                }
                self.snapshots.insert(insert_index, snapshot);
            }
        }
    }

    /// Drop stale snapshots from the collection.
    fn drop_stale_snapshots(&mut self) {
        loop {
            if matches!(self.snapshots.back(), Some(b) if b.time < self.oldest_relevant_snapshot_time)
            {
                self.snapshots.pop_back();
            } else {
                break;
            }
        }
    }

    /// Drop stale snapshots from the collection.
    pub fn update(&mut self) {
        self.drop_stale_snapshots();
    }

    /// Given a timestamp, select the most relevant snapshot.
    /// Update the oldest relevant snapshot.
    pub fn get(&mut self, time: Timestamp) -> SnapshotFetchResult {
        let snaps = &self.snapshots;

        match snaps.len() {
            0 => SnapshotFetchResult::NoData,
            1 => {
                let s = &snaps[0];
                if s.time < time {
                    self.oldest_relevant_snapshot_time = s.time;
                    SnapshotFetchResult::MissingNewer(s.layers.clone())
                } else {
                    // don't update oldest relevant time as we're missing it!
                    SnapshotFetchResult::MissingOlder(s.layers.clone())
                }
            }
            _ => {
                // If we're lagging on snapshots, just draw the most recent one.
                if let Some(s) = snaps.front() {
                    if s.time < time {
                        self.oldest_relevant_snapshot_time = s.time;
                        return SnapshotFetchResult::MissingNewer(s.layers.clone());
                    }
                }
                // Find the two snapshots that bracket the requested timestamp.
                for (newer, older) in snaps.iter().zip(snaps.iter().skip(1)) {
                    if time <= newer.time && time >= older.time {
                        self.oldest_relevant_snapshot_time = older.time;
                        return SnapshotFetchResult::Good(newer.layers.clone());
                    }
                }
                SnapshotFetchResult::Error(Vec::from(snaps.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tunnels_lib::{ArcSegment, Snapshot};

    use super::*;
    use std::iter::Iterator;
    use std::sync::Arc;
    use tunnels_lib::arc_segment_for_test;

    fn mksnapshot(n: u64, time: Timestamp) -> Snapshot {
        Snapshot {
            frame_number: n,
            time,
            layers: Vec::new(),
        }
    }

    fn mksnapshot_with_arc(n: u64, time: Timestamp, arc: ArcSegment) -> Snapshot {
        let mut snap = mksnapshot(n, time);
        snap.layers.push(Arc::new(vec![arc]));
        snap
    }

    fn zip_assert_same<A: Eq, T, U>(a: T, b: U)
    where
        T: IntoIterator<Item = A>,
        U: IntoIterator<Item = A>,
    {
        for (ai, bi) in a.into_iter().zip(b.into_iter()) {
            assert!(ai == bi);
        }
    }

    #[test]
    fn test_insert_snapshot() {
        let mut sm = SnapshotManager::default();
        let snapshots_ordered = [
            mksnapshot(0, Timestamp(10000)),
            mksnapshot(1, Timestamp(20000)),
            mksnapshot(2, Timestamp(30000)),
        ];
        for s in &snapshots_ordered {
            sm.insert_snapshot(s.clone());
        }

        zip_assert_same(sm.snapshots.iter(), snapshots_ordered.iter().rev());

        let unordered_snapshot = mksnapshot(3, Timestamp(15000));
        sm.insert_snapshot(unordered_snapshot);

        let correct_ordering = [30000, 20000, 15000, 10000];

        zip_assert_same(sm.snapshots.iter().map(|s| &s.time.0), &correct_ordering);
    }

    #[test]
    fn test_drop_stale() {
        let mut sm = SnapshotManager::default();
        let snaps = [
            mksnapshot(0, Timestamp(0)),
            mksnapshot(1, Timestamp(1000)),
            mksnapshot(2, Timestamp(2000)),
        ];
        for s in &snaps {
            sm.insert_snapshot(s.clone());
        }
        sm.oldest_relevant_snapshot_time = Timestamp(2000);
        sm.drop_stale_snapshots();

        assert!(sm.snapshots.len() == 1);
        assert!(sm.snapshots[0].time.0 == 2000);
    }

    #[test]
    fn test_no_data() {
        let mut sm = SnapshotManager::default();
        if let SnapshotFetchResult::NoData = sm.get(Timestamp(0)) {
        } else {
            panic!();
        }
    }

    #[test]
    fn test_one_older_frame() {
        let mut sm = SnapshotManager::default();
        let snap = mksnapshot_with_arc(0, Timestamp(0), arc_segment_for_test(0.2, 0.3));
        sm.insert_snapshot(snap.clone());
        if let SnapshotFetchResult::MissingNewer(f) = sm.get(Timestamp(1000)) {
            assert_eq!(snap.layers, f);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_one_newer_frame() {
        let mut sm = SnapshotManager::default();
        let snap = mksnapshot_with_arc(0, Timestamp(10000), arc_segment_for_test(0.2, 0.3));
        sm.insert_snapshot(snap.clone());
        if let SnapshotFetchResult::MissingOlder(f) = sm.get(Timestamp(1000)) {
            assert_eq!(snap.layers, f);
        } else {
            panic!();
        }
    }

    fn setup_two_frame_test() -> (SnapshotManager, Snapshot, Snapshot) {
        let mut sm = SnapshotManager::default();
        let snap0 = mksnapshot_with_arc(0, Timestamp(0), arc_segment_for_test(0.2, 0.3));
        let snap1 = mksnapshot_with_arc(1, Timestamp(10000), arc_segment_for_test(0.2, 0.3));
        sm.insert_snapshot(snap0.clone());
        sm.insert_snapshot(snap1.clone());
        (sm, snap0, snap1)
    }

    #[test]
    fn test_two_frames_exact_newer() {
        let (mut sm, _snap0, snap1) = setup_two_frame_test();
        if let SnapshotFetchResult::Good(f) = sm.get(Timestamp(1000)) {
            assert_eq!(snap1.layers, f);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_two_frames_exact_older() {
        let (mut sm, snap0, _snap1) = setup_two_frame_test();
        if let SnapshotFetchResult::Good(f) = sm.get(Timestamp(0)) {
            assert_eq!(snap0.layers, f);
        } else {
            panic!();
        }
    }
}
