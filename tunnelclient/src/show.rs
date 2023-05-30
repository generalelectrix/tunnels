use crate::config::ClientConfig;
use crate::draw::Draw;
use crate::snapshot_manager::SnapshotFetchResult::*;
use crate::snapshot_manager::SnapshotManager;
use crate::snapshot_manager::SnapshotManagerHandle;
use crate::timesync::SynchronizerHandle;
use crate::timesync::{Client as TimesyncClient, Synchronizer};
use anyhow::{anyhow, Context as ErrorContext, Result};
use graphics::clear;
use log::{debug, error, info, max_level, warn, Level};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::*;
use sdl2_window::Sdl2Window;
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
    render_logger: RenderIssueLogger,
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
        let snapshot_manager = Arc::new(Mutex::new(SnapshotManager::default()));
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
        .samples(if cfg.anti_alias { 4 } else { 0 })
        .fullscreen(cfg.fullscreen)
        .build()
        .map_err(|err| anyhow!("{err}"))?;

        window.set_capture_cursor(cfg.capture_mouse);
        window.set_max_fps(120);

        Ok(Show {
            gl: GlGraphics::new(opengl),
            snapshot_manager,
            timesync: synchronizer,
            cfg,
            run_flag,
            window,
            render_logger: RenderIssueLogger::new(Duration::from_secs(1)),
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
        // Get frame from the snapshot service.

        let delayed_time = match self.timesync.lock() {
            Err(_) => {
                // The timesync update thread has panicked, abort the show.
                self.run_flag.stop();
                error!("Timesync service crashed; aborting show.");
                return;
            }
            Ok(ref mut ts) => ts.now() - Timestamp::from_duration(self.cfg.render_delay),
        };

        let frame = match self.snapshot_manager.lock().unwrap().get(delayed_time) {
            NoData => {
                self.render_logger
                    .log(delayed_time, "No data available from snapshot service.");
                return;
            }
            Error(snaps) => {
                let snap_times = snaps.iter().map(|s| s.time).collect::<Vec<_>>();
                error!(
                    "Something went wrong with snapshot interpolation for time {}.\n{:?}\n",
                    delayed_time, snap_times
                );
                return;
            }
            Good(layers) => layers,
            MissingNewer(layers) => {
                self.render_logger
                    .log(delayed_time, "Interpolation had no newer layer.");
                layers
            }
            MissingOlder(layers) => {
                self.render_logger
                    .log(delayed_time, "Interpolation had no older layer");
                layers
            }
        };

        self.gl.draw(args.viewport(), |c, gl| {
            // Clear the screen.
            clear([0.0, 0.0, 0.0, 1.0], gl);

            // Draw everything.
            frame.draw(&c, gl, &self.cfg);
        });
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

/// Logging helper that either logs everything at debug level or occasionally logs at warn level.
struct RenderIssueLogger {
    interval: Duration,
    last_logged: Timestamp,
    missed: u32,
    log_all: bool,
}

impl RenderIssueLogger {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_logged: Timestamp(0),
            missed: 0,
            log_all: max_level() >= Level::Debug,
        }
    }

    fn log(&mut self, now: Timestamp, msg: &str) {
        if self.log_all {
            debug!("{}", msg);
            return;
        }
        self.missed += 1;

        if now > self.last_logged + Timestamp::from_duration(self.interval) {
            let dt = now - self.last_logged;
            self.last_logged = now;
            warn!(
                "Missed {} snapshots in the last {} seconds.",
                self.missed,
                dt.0 as f64 / 1_000_000.
            );
            self.missed = 0;
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
    timesync: SynchronizerHandle,
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
