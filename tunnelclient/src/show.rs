
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
use std::error::Error;
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
    window: PistonWindow<Sdl2Window>,
}

impl Show {
    pub fn new(cfg: ClientConfig, ctx: &mut Context, run_flag: RunFlag) -> Result<Self, Box<Error>> {
        println!("Running on video channel {}.", cfg.video_channel);

        // Start up the timesync service.
        let mut timesync_client = TimesyncClient::new(&cfg.server_hostname, ctx)?;

        // Synchronize timing with master host.
        println!(
            "Synchronizing timing.  This will take about {} seconds.",
            timesync_client.synchronization_duration().as_secs());

        let synchronizer = Synchronizer::new(timesync_client.synchronize()?);

        println!("Synchronized.");

        // Spin off another thread to periodically update our host time synchronization.
        let timesync_period = cfg.timesync_interval.clone();
        let timesync = Arc::new(Mutex::new(synchronizer));
        let timesync_remote = timesync.clone();
        let timesync_run_flag = run_flag.clone();

        thread::Builder::new().name("timesync".to_string()).spawn(move || {
            while timesync_run_flag.should_run() {
                thread::sleep(timesync_period);
                match timesync_client.synchronize() {
                    Ok(sync) => {
                        let new_estimate = sync.now_as_timestamp();
                        let mut synchronizer = timesync_remote.lock().expect("Timesync mutex poisoned.");
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
            println!("Timesync service shutting down.");
        }).map_err(|e| format!("Timesync service thread failed to spawn: {}", e))?;

        // Set up snapshot reception and management.
        let snapshot_queue: Receiver<Snapshot> =
            SubReceiver::new(&cfg.server_hostname, 6000, cfg.video_channel.to_string().as_bytes(), ctx)?
                .run_async();

        let snapshot_manager = SnapshotManager::new(snapshot_queue);

        let opengl = OpenGL::V3_2;

        // Sleep for a render delay to make sure we have snapshots before we start rendering.
        thread::sleep(Duration::from_millis(cfg.render_delay as u64));

        // Create the window.
        let mut window: PistonWindow<Sdl2Window> = WindowSettings::new(
            format!("tunnelclient: channel {}", cfg.video_channel),
            [cfg.x_resolution, cfg.y_resolution]
        )
            .opengl(opengl)
            .exit_on_esc(true)
            .vsync(true)
            .samples(if cfg.anti_alias { 4 } else { 0 })
            .fullscreen(cfg.fullscreen)
            .build()?;

        window.set_capture_cursor(cfg.capture_mouse);
        window.set_max_fps(120);

        Ok(Show {
            opengl,
            gl: GlGraphics::new(opengl),
            snapshot_manager,
            timesync,
            cfg,
            run_flag,
            window,
        })

    }

    /// Run the show's event loop.
    pub fn run(&mut self) {

        // Run the event loop.
        while let Some(e) = self.window.next() {

            if !self.run_flag.should_run() {
                println!("Quit flag tripped, ending show.");
                break
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
            Err(e) => {
                // The timesync update thread has panicked, abort the show.
                self.run_flag.stop();
                println!("Timesync service crashed; aborting show.");
                return
            },
            Ok(ref mut ts) => ts.now_as_timestamp() - self.cfg.render_delay as f64,
        };

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
        self.timesync.lock().expect("Timesync mutex poisoned").update(dt);
    }
}