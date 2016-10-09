//! Synchronize time between the master and this client.
//! Using this simple technique:
//! http://www.mine-control.com/zack/timesync/timesync.html

use receive::{Receive};
use serde::Deserialize;
use std::time::{Instant, Duration, SystemTime};
use std::thread::sleep;
use stats::{mean, stddev};
use zmq;
use zmq::{Context, Socket, DONTWAIT};

pub type Timestamp = f64;

const SNTP_PORT: u64 = 8989;

fn f64_to_duration(v: f64) -> Duration {
    let secs = v.floor();
    let nanos = (v - secs) * 1_000_000_000.0;
    Duration::new(secs as u64, nanos as u32)
}

fn duration_to_f64(dur: Duration) -> f64 {
    dur.as_secs() as f64 + dur.subsec_nanos() as f64 / 1_000_000_000.0
}

/// Interact with our homebrew quasi-SNTP service.
/// Not a lot of error handling in here.  This service runs at startup and
/// if it isn't successful we have to bail regardless.
struct SntpClient {
    socket: Socket
}

impl SntpClient {
    /// Create a new 0mq SUB connected to the provided socket addr.
    fn new(host: &str, port: u64, ctx: &mut Context) -> Self {
        let mut socket = ctx.socket(zmq::REQ).unwrap();
        let addr = format!("tcp://{}:{}", host, port);
        socket.connect(&addr).unwrap();

        SntpClient{socket: socket}
    }

    /// Take a time delay measurement.
    fn take_measurement(&mut self) -> SntpMeasurement {
        let now = Instant::now();
        self.socket.send(&[], 0).unwrap();
        let buf = self.receive_buffer(true).unwrap();
        let elapsed = now.elapsed();
        let timestamp: f64 = self.deserialize_msg(buf).unwrap();
        SntpMeasurement{sent: now, round_trip: elapsed, timestamp: timestamp}
    }
}

impl Receive for SntpClient {
    fn receive_buffer(&mut self, block: bool) -> Option<Vec<u8>> {
        let flag = if block {0} else {DONTWAIT};
        if let Ok(b) = self.socket.recv_bytes(flag) {Some(b)}
        else {None}
    }
}

#[derive(Debug)]
struct SntpMeasurement {
    sent: Instant,
    round_trip: Duration,
    timestamp: Timestamp
}

#[derive(Debug)]
pub struct SntpSync {
    ref_time: Instant,
    host_ref_time: Timestamp
}

impl SntpSync {
    /// Return our estimate of what time it is now on the host.
    /// This is in milliseconds.
    pub fn now_as_timestamp(&self) -> Timestamp {
        let time_secs = self.host_ref_time + duration_to_f64(self.ref_time.elapsed());
        time_secs * 1000.0
    }
}

/// Get the offset between this machine's system clock and the host's.
pub fn synchronize(host: &str, poll_period: Duration, n_meas: usize) -> SntpSync {
    let mut ctx = Context::new();
    let reference_time = Instant::now();
    let mut req = SntpClient::new(host, SNTP_PORT, &mut ctx);
    // Take a bunch of measurements, sleeping in between.
    let mut measurements =
        (0..n_meas)
        .map(|_| {
            let m = req.take_measurement();
            sleep(poll_period);
            m
        })
        .collect::<Vec<_>>();
    // Sort the measurements by round-trip time and remove outliers.
    measurements.sort_by_key(|m| m.round_trip);
    let median_delay = measurements[(n_meas / 2) as usize].round_trip;
    let stddev = stddev(measurements.iter().map(|m| duration_to_f64(m.round_trip)));
    let cutoff = f64_to_duration(duration_to_f64(median_delay) + stddev);

    measurements.retain(|m| m.round_trip < cutoff);

    if measurements.len() < n_meas / 2 {
        panic!("Ony got {} synchronization samples.", measurements.len());
    }

    // Estimate the remote clock time that corresponds to our reference time.
    let remote_time_estimates =
        measurements.iter()
        .map(|m| {
            let delta = (m.sent + m.round_trip / 2).duration_since(reference_time);
            m.timestamp - duration_to_f64(delta)
        });
    // Take the average of these estaimtes, and we're done
    let best_remote_time_estimate = mean(remote_time_estimates);
    SntpSync{ref_time: reference_time, host_ref_time: best_remote_time_estimate}
}

#[test]
fn test_duration_f64_round_trip() {
    let now = Instant::now();
    sleep(Duration::from_millis(100));
    let delta = now.elapsed();
    let rt = f64_to_duration(duration_to_f64(delta));
    println!("delta: {:?}, rt: {:?}", delta, rt);
    assert!(delta == rt);
}

// This test requires the remote SNTP service to be running.
#[test]
#[ignore]
fn test_synchronize() {
    let sync = synchronize("localhost", Duration::from_millis(500), 10);
    println!("Ref time: {:?}, remote estimate: {}", sync.ref_time, sync.host_ref_time);
}
