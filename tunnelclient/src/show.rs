
use config::ClientConfig;
use graphics::clear;
use opengl_graphics::{ GlGraphics, OpenGL };
use piston_window::*;
use receive::{SubReceiver, Snapshot};
use timesync::{Client as TimesyncClient, Synchronizer};
use glutin_window::GlutinWindow;
use sdl2_window::Sdl2Window;
use std::time::Duration;
use std::sync::mpsc::Receiver;
use std::thread;
use std::sync::{Arc, Mutex};
use draw::Draw;
use zmq::Context;
use snapshot_manager::{SnapshotManager, SnapshotUpdateError};
use snapshot_manager::InterpResult::*;
use utils::RunFlag;


/// Top-level structure that owns all of the show data.
pub struct Show {
    opengl: OpenGL,
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManager,
    timesync: Arc<Mutex<Synchronizer>>,
    cfg: ClientConfig,
    run_flag: RunFlag,
}

impl Show {
    pub fn new(cfg: ClientConfig, ctx: &mut Context, run_flag: RunFlag) -> Self {

        // Start up the timesync service.
        let mut timesync_client = TimesyncClient::new(&cfg.server_hostname, ctx);

        // Synchronize timing with master host.
        println!(
            "Synchronizing timing.  This will take about {} seconds.",
            timesync_client.synchronization_duration().as_secs());

        let synchronizer = Synchronizer::new(timesync_client.synchronize().unwrap());

        // Spin off another thread to periodically update our host time synchronization.
        let timesync_period = cfg.timesync_interval.clone();
        let timesync = Arc::new(Mutex::new(synchronizer));
        let timesync_remote = timesync.clone();
        let timesync_run_flag = run_flag.clone();

        thread::spawn(move || {

            while timesync_run_flag.should_run() {

                thread::sleep(timesync_period);
                match timesync_client.synchronize() {
                    Ok(sync) => {
                        let new_estimate = sync.now_as_timestamp();
                        let mut synchronizer = timesync_remote.lock().unwrap();
                        let old_estimate = synchronizer.now_as_timestamp();
                        println!(
                            "Updating time sync.  Change from previous estimate: {}",
                            new_estimate - old_estimate);
                        synchronizer.update_current(sync);
                    },
                    Err(e) => {
                        println!("{}", e);
                    }
                }
            }
        });

        // Set up snapshot reception and management.
        let snapshot_queue: Receiver<Snapshot> =
            SubReceiver::new(&cfg.server_hostname, 6000, cfg.video_channel.as_bytes(), ctx)
                .run_async();

        let snapshot_manager = SnapshotManager::new(snapshot_queue);

        let opengl = OpenGL::V3_2;

        Show {
            opengl,
            gl: GlGraphics::new(opengl),
            snapshot_manager,
            timesync,
            cfg,
            run_flag,
        }
    }

    /// Create this show's window, create time synchronization, and run the show's event loop.
    pub fn run(&mut self) {

        // Sleep for a render delay to make sure we have snapshots before we start rendering.
        thread::sleep(Duration::from_millis(self.cfg.render_delay as u64));

        // Create the window.
        let mut window: PistonWindow<Sdl2Window> = WindowSettings::new(
            format!("tunnelclient: channel {}", self.cfg.video_channel),
            [self.cfg.x_resolution, self.cfg.y_resolution]
        )
            .opengl(self.opengl)
            .exit_on_esc(true)
            .vsync(true)
            .samples(if self.cfg.anti_alias {4} else {0})
            .fullscreen(self.cfg.fullscreen)
            .build()
            .unwrap();

        window.set_capture_cursor(true);
        window.set_max_fps(120);

        // Run the event loop.
        while let Some(e) = window.next() {

            if !self.run_flag.should_run() {
                break
            }

            if let Some(update_args) = e.update_args() {
                self.update(update_args.dt);
            }

            if let Some(r) = e.render_args() {
                self.render(&r);
            }
        }
    }

    /// Render a frame to the window.
    fn render(&mut self, args: &RenderArgs) {

        // Get frame interpolation from the snapshot service.
        let delayed_time =
            self.timesync.lock().unwrap().now_as_timestamp()
                - self.cfg.render_delay as f64;

        let (msg, maybe_frame) = match self.snapshot_manager.get_interpolated(delayed_time) {
            NoData => (Some("No data available from snapshot service.".to_string()), None),
            Error(snaps) => {
                let snap_times = snaps.iter().map(|s| s.time).collect::<Vec<_>>();
                let msg = format!(
                    "Something went wrong with snapshot interpolation for time {}.\n{:?}\n",
                    delayed_time,
                    snap_times);
                (Some(msg), None)
            },
            Good(layers) => (None, Some(layers)),
            MissingNewer(layers) => (Some("Interpolation had no newer layer.".to_string()), Some(layers)),
            MissingOlder(layers) => (Some("Interpolation had no older layer".to_string()), Some(layers))
        };
        if let Some(m) = msg { println!("{}", m); };

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
                SnapshotUpdateError::Disconnected => "disconnected"
            };
            println!("An error occurred during snapshot update: {:?}", msg);
        }
        // Update the interpolation parameter on our time synchronization.
        self.timesync.lock().unwrap().update(dt);
    }
}