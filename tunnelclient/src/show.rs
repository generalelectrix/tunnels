use crate::config::ClientConfig;
use crate::config::SnapshotManagement;
use crate::draw::Draw;
use crate::snapshot_manager::SingleSnapshotManager;
use crate::snapshot_manager::SnapshotFetchResult;

use crate::snapshot_manager::SnapshotManager;
use crate::snapshot_manager::SnapshotManagerHandle;
use crate::snapshot_manager::VecDequeSnapshotManager;
use crate::timesync::SynchronizerHandle;
use crate::timesync::{Client as TimesyncClient, Synchronizer};
use anyhow::{anyhow, Context as ErrorContext, Result};
use graphics::clear;
use log::{debug, error, info, warn};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::*;
use sdl2_window::Sdl2Window;
use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tunnels_lib::RunFlag;
use tunnels_lib::Snapshot;
use tunnels_lib::Timestamp;
use zero_configure::pub_sub::Receiver;
use zmq::Context;

/// Top-level structure that owns all of the show data.
pub struct Show {
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManagerHandle,
    timesync: SynchronizerHandle,
    cfg: ClientConfig,
    run_flag: RunFlag,
    window: PistonWindow<Sdl2Window>,
    render_reporter: RenderIssueLogger,
}

impl Show {
    pub fn new(cfg: ClientConfig, ctx: Context, run_flag: RunFlag) -> Result<Self> {
        info!("Running on video channel {}.", cfg.video_channel);

        // Start up the timesync service.
        let mut timesync_client = TimesyncClient::new(&cfg.server_hostname, ctx.clone())?;

        // Synchronize timing with master host.
        info!(
            "Synchronizing timing.  This will take about {} seconds.",
            timesync_client.synchronization_duration().as_secs()
        );

        let synchronizer = Arc::new(Mutex::new(Synchronizer::new(
            timesync_client.synchronize()?,
        )));
        info!("Synchronized.");

        // Spin off another thread to periodically update our host time synchronization.
        update_timesync(
            cfg.timesync_interval,
            timesync_client,
            synchronizer.clone(),
            run_flag.clone(),
        )?;

        // Set up snapshot reception and management.
        let snapshot_manager = Arc::new(Mutex::new(match cfg.snapshot_management {
            SnapshotManagement::Queued => {
                Box::<VecDequeSnapshotManager>::default() as Box<dyn SnapshotManager>
            }
            SnapshotManagement::Single => {
                Box::<SingleSnapshotManager>::default() as Box<dyn SnapshotManager>
            }
        }));
        receive_snapshots(
            &ctx,
            &cfg,
            snapshot_manager.clone(),
            synchronizer.clone(),
            run_flag.clone(),
        )?;

        let opengl = OpenGL::V3_2;

        // Sleep for a render delay to make sure we have snapshots before we start rendering.
        thread::sleep(cfg.render_delay);

        // Create the window.
        let mut window: PistonWindow<Sdl2Window> = WindowSettings::new(
            format!("tunnelclient: channel {}", cfg.video_channel),
            [cfg.x_resolution, cfg.y_resolution],
        )
        .graphics_api(opengl)
        .exit_on_esc(true)
        .vsync(true)
        .samples(4)
        .fullscreen(cfg.fullscreen)
        .build()
        .map_err(|err| anyhow!("{err}"))?;

        window.set_capture_cursor(cfg.capture_mouse);

        Ok(Show {
            gl: GlGraphics::new(opengl),
            snapshot_manager,
            timesync: synchronizer,
            cfg,
            run_flag,
            window,
            render_reporter: RenderIssueLogger::new(
                Duration::from_secs(1),
                Box::new(log_render_issue_reporter),
            ),
        })
    }

    /// Run the show's event loop.
    pub fn run(&mut self) {
        // Run the event loop.
        while let Some(e) = self.window.next() {
            if !self.run_flag.should_run() {
                info!("Quit flag tripped, ending show.");
                break;
            }

            if let Some(update_args) = e.update_args() {
                self.update(update_args.dt);
            }

            if let Some(r) = e.render_args() {
                self.render(&r);
            }
        }

        // If the window is closed, the event loop will exit normally.  Flip the run flag to stop
        // to ensure all of the services close down and we don't leak a timesync thread.
        // TODO: hold onto the join handle for the timesync service?
        self.run_flag.stop();
    }

    /// Render a frame to the window.
    fn render(&mut self, args: &RenderArgs) {
        // Get our best estimate of the current host time.
        let now = match self.timesync.lock() {
            Err(_) => {
                // The timesync update thread has panicked, abort the show.
                self.run_flag.stop();
                error!("Timesync service crashed; aborting show.");
                return;
            }
            Ok(ref mut ts) => ts.now(),
        };

        let delayed_time = now - Timestamp::from_duration(self.cfg.render_delay);

        let (snapshot_latency, snapshot_result) = {
            let mut manager = self.snapshot_manager.lock().unwrap();
            let latency = manager.peek_front().map(|snap| (now - snap.time).0);
            let result = manager.get(delayed_time);
            (latency, result)
        };

        if let Some(frame) = snapshot_result.frame() {
            self.gl.draw(args.viewport(), |c, gl| {
                // Clear the screen.
                clear([0.0, 0.0, 0.0, 1.0], gl);

                // Draw everything.
                frame.draw(&c, gl, &self.cfg);
            });
        }

        // Report render issues.
        self.render_reporter
            .report(now, delayed_time, &snapshot_result, snapshot_latency);
    }

    /// Perform a timestep update of all of the state of the show.
    fn update(&mut self, dt: f64) {
        // Update the state of the snapshot manager.
        self.snapshot_manager.lock().unwrap().update();
        // Update the interpolation parameter on our time synchronization.
        self.timesync
            .lock()
            .expect("Timesync mutex poisoned")
            .update(dt);
    }
}

/// Report render issues via logging.
/// Only log if we have missed at least one frame.
fn log_render_issue_reporter(report: RenderIssueReport) {
    if report.missed == 0 {
        return;
    }
    warn!("{}", report);
}

pub type RenderIssueReporter = Box<dyn FnMut(RenderIssueReport)>;

pub struct RenderIssueReport {
    interval: Duration,
    total: usize,
    missed: usize,
    mean_snapshot_latency_us: i64,
    best_snapshot_latency_us: i64,
    worst_snapshot_latency_us: i64,
}

impl Display for RenderIssueReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "missed {}/{} snapshots in the last {} seconds.\nmean latency: {}\nworst latency: {}\nbest latency: {}", self.missed, self.total, self.interval.as_secs_f64(), self.mean_snapshot_latency_us, self.worst_snapshot_latency_us, self.best_snapshot_latency_us)
    }
}

/// Helper to periodically report render quality.
struct RenderIssueLogger {
    interval: Duration,
    last_report: Timestamp,
    reporter: RenderIssueReporter,
    total: usize,
    missed: usize,
    mean_snapshot_latency_us: i64,
    worst_snapshot_latency_us: i64,
    best_snapshot_latency_us: i64,
}

impl RenderIssueLogger {
    fn new(interval: Duration, reporter: RenderIssueReporter) -> Self {
        Self {
            interval,
            last_report: Timestamp(0),
            reporter,
            total: 0,
            missed: 0,
            mean_snapshot_latency_us: 0,
            worst_snapshot_latency_us: i64::MIN,
            best_snapshot_latency_us: i64::MAX,
        }
    }

    fn report(
        &mut self,
        now: Timestamp,
        delayed_time: Timestamp,
        render_result: &SnapshotFetchResult,
        snapshot_latency: Option<i64>,
    ) {
        if let Some(lat) = snapshot_latency {
            self.mean_snapshot_latency_us += lat;
            if lat > self.worst_snapshot_latency_us {
                self.worst_snapshot_latency_us = lat;
            }
            if lat < self.best_snapshot_latency_us {
                self.best_snapshot_latency_us = lat;
            }
        }

        use SnapshotFetchResult::*;
        match render_result {
            NoData => {
                self.missed += 1;
                warn!("No data available from snapshot service.");
            }
            Error(snaps) => {
                let snap_times = snaps.iter().map(|s| s.time).collect::<Vec<_>>();
                error!(
                    "Something went wrong with snapshot interpolation for time {}.\n{:?}\n",
                    delayed_time, snap_times
                );
                self.missed += 1;
            }
            Good(_) => {
                self.total += 1;
            }
            MissingNewer(_) => {
                self.total += 1;
                self.missed += 1;
                debug!("Interpolation had no newer layer.");
            }
            MissingOlder(_) => {
                self.total += 1;
                debug!("Interpolation had no older layer");
            }
        }
        if (now - self.last_report) > Timestamp::from_duration(self.interval) {
            (self.reporter)(RenderIssueReport {
                interval: self.interval,
                total: self.total,
                missed: self.missed,
                mean_snapshot_latency_us: if self.total == 0 {
                    0
                } else {
                    self.mean_snapshot_latency_us / self.total as i64
                },
                worst_snapshot_latency_us: self.worst_snapshot_latency_us,
                best_snapshot_latency_us: self.best_snapshot_latency_us,
            });
            self.last_report = now;
            self.missed = 0;
            self.total = 0;
            self.mean_snapshot_latency_us = 0;
            self.worst_snapshot_latency_us = i64::MIN;
            self.best_snapshot_latency_us = i64::MAX;
        }
    }
}

/// Spawn a thread to receive snapshots.
/// Inject them into the provided manager.
/// The thread runs until the run flag is tripped.
fn receive_snapshots(
    ctx: &Context,
    cfg: &ClientConfig,
    snapshot_manager: SnapshotManagerHandle,
    _timesync: SynchronizerHandle,
    run_flag: RunFlag,
) -> Result<()> {
    let mut receiver: Receiver<Snapshot> = Receiver::new(
        ctx,
        &cfg.server_hostname,
        6000,
        Some(&[cfg.video_channel as u8]),
    )?;
    thread::Builder::new()
        .name("snapshot_receiver".to_string())
        .spawn(move || {
            loop {
                if !run_flag.should_run() {
                    info!("Snapshot receiver shutting down.");
                    break;
                }
                match receiver.receive_msg(true) {
                    Ok(Some(msg)) => {
                        // println!(
                        //     "receive latency: {}",
                        //     timesync.lock().unwrap().now() - msg.time
                        // );
                        snapshot_manager.lock().unwrap().insert_snapshot(msg);
                    }
                    Ok(None) => continue, // Odd case, given that we should have blocked.
                    Err(e) => error!("receive error: {e}"),
                }
            }
        })?;
    Ok(())
}

/// Spawn a thread to periodically update timesync.
fn update_timesync(
    period: Duration,
    mut client: TimesyncClient,
    synchronizer: SynchronizerHandle,
    run_flag: RunFlag,
) -> Result<()> {
    thread::Builder::new()
        .name("timesync".to_string())
        .spawn(move || {
            // FIXME: rather than sleep/flag polling we should use a select
            // mechanism to ensure prompt quit.
            while run_flag.should_run() {
                thread::sleep(period);
                match client.synchronize() {
                    Ok(sync) => {
                        let new_estimate = sync.now();
                        let mut synchronizer =
                            synchronizer.lock().expect("Timesync mutex poisoned.");
                        let old_estimate = synchronizer.now();
                        info!(
                            "Updating time sync.  Change from previous estimate: {}",
                            new_estimate - old_estimate
                        );
                        synchronizer.update_current(sync);
                    }
                    Err(e) => {
                        warn!("{}", e);
                    }
                }
            }
            info!("Timesync service shutting down.");
        })
        .context("timesync service thread failed to spawn")?;
    Ok(())
}
