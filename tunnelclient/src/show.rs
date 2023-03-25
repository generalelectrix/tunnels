use crate::config::ClientConfig;
use crate::draw::Draw;
use crate::receive::SubReceiver;
use crate::snapshot_manager::InterpResult::*;
use crate::snapshot_manager::{SnapshotManager, SnapshotUpdateError};
use crate::timesync::{Client as TimesyncClient, Synchronizer};
use graphics::clear;
use log::{debug, error, info, max_level, warn, Level};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::*;
use sdl2_window::Sdl2Window;
use std::error::Error;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tunnels_lib::RunFlag;
use tunnels_lib::{Snapshot, Timestamp};
use zmq::Context;

/// Top-level structure that owns all of the show data.
pub struct Show {
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManager,
    timesync: Arc<Mutex<Synchronizer>>,
    cfg: ClientConfig,
    run_flag: RunFlag,
    window: PistonWindow<Sdl2Window>,
    render_logger: RenderIssueLogger,
}

impl Show {
    pub fn new(cfg: ClientConfig, ctx: Context, run_flag: RunFlag) -> Result<Self, Box<dyn Error>> {
        info!("Running on video channel {}.", cfg.video_channel);

        // Start up the timesync service.
        let mut timesync_client = TimesyncClient::new(&cfg.server_hostname, ctx.clone())?;

        // Synchronize timing with master host.
        info!(
            "Synchronizing timing.  This will take about {} seconds.",
            timesync_client.synchronization_duration().as_secs()
        );

        let synchronizer = Synchronizer::new(timesync_client.synchronize()?);

        info!("Synchronized.");

        // Spin off another thread to periodically update our host time synchronization.
        let timesync_period = cfg.timesync_interval;
        let timesync = Arc::new(Mutex::new(synchronizer));
        let timesync_remote = timesync.clone();
        let timesync_run_flag = run_flag.clone();

        thread::Builder::new()
            .name("timesync".to_string())
            .spawn(move || {
                // FIXME: rather than sleep/flag polling we should use a select
                // mechanism to ensure prompt quit.
                while timesync_run_flag.should_run() {
                    thread::sleep(timesync_period);
                    match timesync_client.synchronize() {
                        Ok(sync) => {
                            let new_estimate = sync.now();
                            let mut synchronizer =
                                timesync_remote.lock().expect("Timesync mutex poisoned.");
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
            .map_err(|e| format!("Timesync service thread failed to spawn: {}", e))?;

        // Set up snapshot reception and management.
        let snapshot_queue: Receiver<Snapshot> =
            SubReceiver::new(&cfg.server_hostname, 6000, &[cfg.video_channel as u8], ctx)?
                .run_async()?;

        let snapshot_manager = SnapshotManager::new(snapshot_queue);

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
        .build()?;

        window.set_capture_cursor(cfg.capture_mouse);
        window.set_max_fps(120);

        Ok(Show {
            gl: GlGraphics::new(opengl),
            snapshot_manager,
            timesync,
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
        // Get frame interpolation from the snapshot service.

        let delayed_time = match self.timesync.lock() {
            Err(_) => {
                // The timesync update thread has panicked, abort the show.
                self.run_flag.stop();
                error!("Timesync service crashed; aborting show.");
                return;
            }
            Ok(ref mut ts) => ts.now() - Timestamp::from_duration(self.cfg.render_delay),
        };

        let maybe_frame = match self.snapshot_manager.get_interpolated(delayed_time) {
            NoData => {
                self.render_logger
                    .log(delayed_time, "No data available from snapshot service.");
                None
            }
            Error(snaps) => {
                let snap_times = snaps.iter().map(|s| s.time).collect::<Vec<_>>();
                error!(
                    "Something went wrong with snapshot interpolation for time {}.\n{:?}\n",
                    delayed_time, snap_times
                );
                None
            }
            Good(layers) => Some(layers),
            MissingNewer(layers) => {
                self.render_logger
                    .log(delayed_time, "Interpolation had no newer layer.");
                Some(layers)
            }
            MissingOlder(layers) => {
                self.render_logger
                    .log(delayed_time, "Interpolation had no older layer");
                Some(layers)
            }
        };

        if let Some(frame) = maybe_frame {
            let cfg = &self.cfg;

            self.gl.draw(args.viewport(), |c, gl| {
                // Clear the screen.
                clear([0.0, 0.0, 0.0, 1.0], gl);

                // Draw everything.
                frame.draw(&c, gl, cfg);
            });
        }
    }

    /// Perform a timestep update of all of the state of the show.
    fn update(&mut self, dt: f64) {
        // Update the state of the snapshot manager.
        let update_result = self.snapshot_manager.update();
        if let Err(e) = update_result {
            let msg = match e {
                SnapshotUpdateError::Disconnected => "disconnected",
            };
            println!("An error occurred during snapshot update: {:?}", msg);
        }
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
