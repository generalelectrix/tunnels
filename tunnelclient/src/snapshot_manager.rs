//! Handle emptying a queue of snapshots, maintaining a time-ordered collection,
//! and interpolating between them on demand.
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};
use sntp_service::{SntpSync, Timestamp};
use std::time::{Instant, Duration};
use receive::{Snapshot, LayerCollection};
use interpolate::Interpolate;

/// Handle receiving and maintaining a collection of snapshots.
/// Provide interpolated snapshots on request.
pub struct SnapshotManager {
    snapshot_queue: Receiver<Snapshot>,
    snapshots: VecDeque<Snapshot>, // Ordered queue of snapshots; latest is snapshots.front()
    oldest_relevant_snapshot_time: u64
}

pub enum SnapshotUpdateError {Disconnected}

pub enum InterpResult {
    NoData, // no data is available at all
    Good(LayerCollection), // Both snapshots were available.
    MissingNewer(LayerCollection), // Data is out-of-date for current timestamp.
    MissingOlder(LayerCollection), // We only have snapshot data newer than requested.
    Error(Vec<Snapshot>) // Something went wrong and we couldn't perform interpolation.
}

enum InsertStrategy {
    PushFront,
    Insert
}

impl SnapshotManager {
    pub fn new(queue: Receiver<Snapshot>) -> Self {
        SnapshotManager{
            snapshot_queue: queue,
            snapshots: VecDeque::new(),
            oldest_relevant_snapshot_time: 0
        }
    }

    /// Add a new snapshot, ensuring the collection remains ordered.
    fn insert_snapshot(&mut self, snapshot: Snapshot) {
        let insert_strategy = match self.snapshots.front() {
            None => InsertStrategy::PushFront,
            Some(s) => if snapshot.time > s.time {InsertStrategy::PushFront} else {InsertStrategy::Insert}
        };
        match insert_strategy {
            InsertStrategy::PushFront => {self.snapshots.push_front(snapshot);},
            InsertStrategy::Insert => {
                let mut insert_index = 0;
                // iterate backwards and find the right spot to insert
                for (index, older_snapshot) in self.snapshots.iter().enumerate() {
                    if snapshot.time > older_snapshot.time {
                        insert_index = index;
                        break
                    }
                }
                self.snapshots.insert(insert_index, snapshot);
            }
        }

    }

    /// Get the latest snapshot from the queue, if one is available.
    fn get_from_queue(&self) -> Result<Option<Snapshot>, SnapshotUpdateError> {
        match self.snapshot_queue.try_recv() {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(SnapshotUpdateError::Disconnected)
        }
    }

    /// Drain the snapshot queue and store all the results.
    fn drain_queue(&mut self) -> Result<(), SnapshotUpdateError> {
        loop {
            match self.get_from_queue() {
                Ok(Some(snapshot)) => {self.insert_snapshot(snapshot);},
                Ok(None) => return Ok(()),
                Err(e) => return Err(e)
            }
        }
    }

    /// Drop stale snapshots from the collection.
    fn drop_stale_snapshots(&mut self) {
        loop {
            let do_pop = match self.snapshots.back() {
                Some(b) if b.time < self.oldest_relevant_snapshot_time => true,
                _ => false
            };
            if do_pop {self.snapshots.pop_back();}
            else {break}
        }
    }

    /// Drain the snapshot queue of any pending items, and incorporate them into
    /// the collection.  Drop stale snapshots from the collection.
    pub fn update(&mut self) -> Result<(), SnapshotUpdateError> {
        let recv_result = self.drain_queue();
        self.drop_stale_snapshots();
        if self.snapshots.len() > 1 {
            println!("n snaps: {}", self.snapshots.len());
        }
        recv_result
    }

    /// Given a timestamp, interpolate between the two most relevant snapshots.
    /// Update the oldest relevant snapshot.
    pub fn get_interpolated(&mut self, time: Timestamp) -> InterpResult {
        let snaps = &self.snapshots;
        let time_rounded = time.round() as u64; // round to nearest millisecond

        match snaps.len() {
            0 => InterpResult::NoData,
            1 => {
                let snap = &snaps[0];
                if snap.time < time_rounded {
                    self.oldest_relevant_snapshot_time = snap.time;
                    InterpResult::MissingNewer(snap.layers.clone())}
                else {
                    // don't update oldest relevant time as we're missing it!
                    InterpResult::MissingOlder(snap.layers.clone())}
            }
            _ => {
                // If we're lagging on snapshots, just draw the most recent one.
                if let Some(s) = snaps.front() {
                    if s.time < time_rounded {return InterpResult::MissingNewer(s.layers.clone());}
                }
                // Find the two snapshots that bracket the requested timestamp.
                for (newer, older) in snaps.iter().zip(snaps.iter().skip(1)) {
                    let (newer_time, older_time) = (newer.time, older.time);

                    if time_rounded == newer_time {
                        // Exact match!  Just use this snapshot.
                        self.oldest_relevant_snapshot_time = newer_time;
                        return InterpResult::Good(newer.layers.clone());

                    } else if time_rounded == older_time {
                        // Exact match!  Just use this snapshot.
                        self.oldest_relevant_snapshot_time = older_time;
                        return InterpResult::Good(older.layers.clone());

                    } else if time_rounded < newer_time && time_rounded > older_time {
                        let alpha = (time - newer_time as f64) / (newer_time - older_time) as f64;
                        let interpolation_result = older.layers.interpolate_with(&newer.layers, alpha);
                        self.oldest_relevant_snapshot_time = older_time;
                        return InterpResult::Good(interpolation_result);
                    }
                }
                InterpResult::Error(Vec::from(snaps.clone()))
            }
        }
    }
}

mod tests {
    use receive::{Snapshot, ArcSegment};
    use super::*;
    use std::sync::mpsc::{channel, Sender};
    use std::iter::Iterator;
    use super::InterpResult::*;
    use interpolate::Interpolate;

    fn mksnapshot(n: u64, time: u64) -> Snapshot {
        Snapshot{
            frame_number: n,
            time: time,
            layers: Vec::new()
        }
    }

    fn mksnapshot_with_arc(n: u64, time: u64, arc: ArcSegment) -> Snapshot {
        let mut snap = mksnapshot(n, time);
        snap.layers.push(vec!(arc));
        snap
    }

    fn zip_assert_same<A: Eq, T, U>(a: T, b: U) where
            T: IntoIterator<Item=A>,
            U: IntoIterator<Item=A> {
        for (ai, bi) in a.into_iter().zip(b.into_iter()) {
            assert!(ai == bi);
        }
    }

    fn setup_sm() -> (Sender<Snapshot>, SnapshotManager) {
        let (tx, rx) = channel();
        let sm = SnapshotManager::new(rx);
        (tx, sm)
    }

    #[test]
    fn test_insert_snapshot() {
        let (_, mut sm) = setup_sm();
        let snapshots_ordered = [mksnapshot(0, 10), mksnapshot(1, 20), mksnapshot(2, 30)];
        for s in &snapshots_ordered {
            sm.insert_snapshot(s.clone());
        }

        zip_assert_same(
            sm.snapshots.iter(),
            snapshots_ordered.iter().rev());

        let unordered_snapshot = mksnapshot(3, 15);
        sm.insert_snapshot(unordered_snapshot.clone());

        let correct_ordering = [30, 20, 15, 10];

        zip_assert_same(
            sm.snapshots.iter().map(|s| &s.time),
            &correct_ordering);
    }

    #[test]
    fn test_drop_stale() {
        let (_, mut sm) = setup_sm();
        let snaps = [mksnapshot(0, 0), mksnapshot(1, 1), mksnapshot(2, 2)];
        for s in &snaps {sm.insert_snapshot(s.clone());}
        sm.oldest_relevant_snapshot_time = 2;
        sm.drop_stale_snapshots();

        assert!(sm.snapshots.len() == 1);
        assert!(sm.snapshots[0].time == 2);
    }

    #[test]
    fn test_interp_no_data() {
        let (_, mut sm) = setup_sm();
        if let NoData = sm.get_interpolated(0.0) {}
        else {panic!();}
    }

    #[test]
    fn test_interp_one_older_frame() {
        let (_, mut sm) = setup_sm();
        let snap = mksnapshot_with_arc(0, 0, ArcSegment::for_test(0.2, 0.3));
        sm.insert_snapshot(snap.clone());
        if let MissingNewer(f) = sm.get_interpolated(1.0) {
            assert_eq!(snap.layers, f);
        }
        else {panic!();}
    }

    #[test]
    fn test_interp_one_newer_frame() {
        let (_, mut sm) = setup_sm();
        let snap = mksnapshot_with_arc(0, 10, ArcSegment::for_test(0.2, 0.3));
        sm.insert_snapshot(snap.clone());
        if let MissingOlder(f) = sm.get_interpolated(1.0) {
            assert_eq!(snap.layers, f);
        }
        else {panic!();}
    }

    fn setup_two_frame_test() -> (SnapshotManager, Snapshot, Snapshot) {
        let (_, mut sm) = setup_sm();
        let snap0 = mksnapshot_with_arc(0, 0, ArcSegment::for_test(0.2, 0.3));
        let snap1 = mksnapshot_with_arc(1, 10, ArcSegment::for_test(0.2, 0.3));
        sm.insert_snapshot(snap0.clone());
        sm.insert_snapshot(snap1.clone());
        (sm, snap0, snap1)
    }

    #[test]
    fn test_interp_two_frames_exact_newer() {
        let (mut sm, snap0, snap1) = setup_two_frame_test();
        if let Good(f) = sm.get_interpolated(10.0) {
            assert_eq!(snap1.layers, f);
        }
        else {panic!();}
    }

    #[test]
    fn test_interp_two_frames_exact_older() {
        let (mut sm, snap0, snap1) = setup_two_frame_test();
        if let Good(f) = sm.get_interpolated(0.0) {
            assert_eq!(snap0.layers, f);
        }
        else {panic!();}
    }

    #[test]
    fn test_interp_two_frames_middle() {
        let (mut sm, snap0, snap1) = setup_two_frame_test();
        if let Good(f) = sm.get_interpolated(5.0) {
            assert_eq!(snap0.layers.interpolate_with(&snap1.layers, 0.0), f);
        }
        else {panic!();}
    }

}