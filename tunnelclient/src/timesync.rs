//! Synchronize time between the master and this client.
//! Using this simple technique:
//! http://www.mine-control.com/zack/timesync/timesync.html

use receive::{Receive};
use std::time::{Instant, Duration};
use std::thread::sleep;
use std::error::Error;
use std::mem;
use stats::{mean, stddev};
use zmq;
use zmq::{Context, Socket, DONTWAIT};
use interpolation::lerp;

pub type Timestamp = f64;

const PORT: u64 = 8989;

/// Convert floating-point seconds into a Duration.
fn f64_to_duration(v: f64) -> Duration {
    let secs = v.floor();
    let nanos = (v - secs) * 1_000_000_000.0;
    Duration::new(secs as u64, nanos as u32)
}

/// Convert a Duration into floating-point seconds.
fn duration_to_f64(dur: Duration) -> f64 {
    dur.as_secs() as f64 + dur.subsec_nanos() as f64 / 1_000_000_000.0
}

/// Provide estimates of the offset between this host's monotonic clock and the server's.
pub struct Client {
    socket: Socket,
    /// Wait this long between individual time offset measurements.
    pub poll_period: Duration,
    /// Make this many measurements in each determination of the time offset.
    pub n_meas: usize,
}

impl Client {
    /// Create a new 0mq REQ connected to the provided socket addr.
    pub fn new(host: &str, ctx: &mut Context) -> Result<Self, Box<dyn Error>> {
        let socket = ctx.socket(zmq::REQ)?;
        let addr = format!("tcp://{}:{}", host, PORT);
        socket.connect(&addr)?;

        Ok(Client {socket, poll_period: Duration::from_millis(500), n_meas: 10})
    }

    /// Return an estimate of how long a synchronization will take.
    pub fn synchronization_duration(&self) -> Duration {
        self.poll_period * self.n_meas as u32
    }

    /// Take a time delay measurement.
    fn measure(&mut self) -> Result<Measurement, Box<dyn Error>> {
        let now = Instant::now();
        self.socket.send(&[][..], 0)?;
        let buf = match self.receive_buffer(true) {
            Some(buf) => buf,
            None => bail!("Unable to receive a response from timesync server.")
        };
        let elapsed = now.elapsed();
        let timestamp: f64 = self.deserialize_msg(buf)?;
        Ok(Measurement {sent: now, round_trip: elapsed, timestamp})
    }

    /// Get the offset between this machine's system clock and the host's.
    pub fn synchronize(&mut self) -> Result<Timesync, Box<dyn Error>> {
        let reference_time = Instant::now();
        // Take a bunch of measurements, sleeping in between.
        let mut measurements = Vec::with_capacity(self.n_meas);
        for _ in 0..self.n_meas {
            measurements.push(self.measure()?);
            sleep(self.poll_period);
        }

        // Sort the measurements by round-trip time and remove outliers.
        measurements.sort_by_key(|m| m.round_trip);
        let median_delay = measurements[(self.n_meas / 2) as usize].round_trip;
        let stddev = stddev(measurements.iter().map(|m| duration_to_f64(m.round_trip)));
        let cutoff = f64_to_duration(duration_to_f64(median_delay) + stddev);

        measurements.retain(|m| m.round_trip < cutoff);

        if measurements.len() < self.n_meas / 2 {
            bail!(format!("Only got {} usable synchronization samples.", measurements.len()));
        }

        // Estimate the remote clock time that corresponds to our reference time.
        let remote_time_estimates =
            measurements.iter()
                .map(|m| {
                    let delta = (m.sent + m.round_trip / 2).duration_since(reference_time);
                    m.timestamp - duration_to_f64(delta)
                });
        // Take the average of these estimates, and we're done
        let best_remote_time_estimate = mean(remote_time_estimates);
        Ok(Timesync { ref_time: reference_time, host_ref_time: best_remote_time_estimate })
    }
}

impl Receive for Client {
    fn receive_buffer(&mut self, block: bool) -> Option<Vec<u8>> {
        let flag = if block {0} else {DONTWAIT};
        if let Ok(b) = self.socket.recv_bytes(flag) {Some(b)}
        else {None}
    }
}

#[derive(Debug)]
struct Measurement {
    sent: Instant,
    round_trip: Duration,
    timestamp: Timestamp
}

#[derive(Debug, Clone)]
pub struct Timesync {
    ref_time: Instant,
    host_ref_time: Timestamp
}

impl Timesync {
    /// Return an estimate of what time it is now on the host.
    /// This is in milliseconds.
    pub fn now_as_timestamp(&self) -> Timestamp {
        let time_secs = self.host_ref_time + duration_to_f64(self.ref_time.elapsed());
        time_secs * 1000.0
    }
}

/// Provide smoothed estimates of the current time on the host.
/// Ensures that we don't suddenly draw a jerk when we update our estimate of the host time offset.
#[derive(Debug, Clone)]
pub struct Synchronizer {
    /// Previous estimate of time on the host.
    last: Timesync,
    /// Most up-to-date estimate of time on the host.
    current: Timesync,
    /// Linear interpolation parameter on [0.0, 1.0].
    alpha: f64,
}

impl Synchronizer {
    /// Instantiate a new synchronizer from an initial time estimate on the host.
    pub fn new(sync: Timesync) -> Self {
        Synchronizer {
            last: sync.clone(),
            current: sync,
            alpha: 1.0,
        }
    }

    /// Update the current estimate and reset the interpolation parameter to 0.
    pub fn update_current(&mut self, sync: Timesync) {
        mem::swap(&mut self.last, &mut self.current);
        self.current = sync;
        self.alpha = 0.0;
    }

    /// Update the interpolation parameter during state update.
    /// Sole argument is the update interval in seconds.
    /// Smooth the host time update over one second by advancing alpha by dt and clamping to 1.0.
    pub fn update(&mut self, dt: f64) {
        self.alpha += dt;
        if self.alpha >= 1.0 {
            self.alpha = 1.0;
        }
    }

    /// Get a (possibly interpolated) estimate of the time on the host.
    pub fn now_as_timestamp(&mut self) -> Timestamp {
        let current = self.current.now_as_timestamp();
        if self.alpha == 1.0 {
            current
        }
        else {
            let old = self.last.now_as_timestamp();
            lerp(&old, &current, &self.alpha)
        }
    }
}

#[test]
fn test_duration_f64_round_trip() {
    let now = Instant::now();
    sleep(Duration::from_millis(100));
    let delta = now.elapsed();
    let rt = f64_to_duration(duration_to_f64(delta));
    println!("delta: {:?}, rt: {:?}", delta, rt);
    assert_eq!(delta, rt);
}

// This test requires the remote timesync service to be running.
#[test]
#[ignore]
fn test_synchronize() {
    let mut client = Client::new("localhost", &mut Context::new()).unwrap();
    let sync = client.synchronize().expect("Test: synchronization failed");
    println!("Ref time: {:?}, remote estimate: {}", sync.ref_time, sync.host_ref_time);
}
