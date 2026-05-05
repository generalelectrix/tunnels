use anyhow::{Result, anyhow};
use client_lib::config::ClientConfig;
use graphics::{CircleArc, Context, clear};
use log::{error, info};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tunnelclient::draw::Draw;
use tunnels_lib::RunFlag;
use tunnels_lib::Snapshot;

pub type SnapshotManagerHandle = Arc<Mutex<Option<SnapshotHandle>>>;
pub type SnapshotHandle = Arc<Snapshot>;

/// Top-level structure that owns all of the show data.
pub struct Show {
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManagerHandle,
    cfg: ClientConfig,
    run_flag: RunFlag,
    window: PistonWindow<Sdl2Window>,
    /// Reference instant for animating the waiting-for-snapshot spinner.
    start_time: Instant,
}

impl Show {
    pub fn new(cfg: ClientConfig, run_flag: RunFlag) -> Result<Self> {
        info!("Running on video channel {}.", cfg.video_channel);

        // Set up snapshot reception and management.
        let snapshot_manager = Arc::new(Mutex::new(None));
        receive_snapshots(&cfg, snapshot_manager.clone(), run_flag.clone());

        let opengl = OpenGL::V3_2;

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
        // This has no effect if vsync is properly enabled, but on machines with
        // broken vsync this does work to make rendering a lot smoother.
        window.set_max_fps(120);

        Ok(Show {
            gl: GlGraphics::new(opengl),
            snapshot_manager,
            cfg,
            run_flag,
            window,
            start_time: Instant::now(),
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

            if let Some(r) = e.render_args() {
                self.render(&r);
            }
        }

        self.run_flag.stop();
    }

    /// Render a frame to the window.
    ///
    /// Always clears to black, then either draws the latest snapshot's
    /// layers or — if no snapshot has arrived yet — a small spinner
    /// indicating the client is up and waiting. The unconditional clear
    /// is what keeps an unfed client from showing uninitialized GPU
    /// memory as static gray noise.
    fn render(&mut self, args: &RenderArgs) {
        let snapshot = self.snapshot_manager.lock().unwrap().clone();
        self.gl.draw(args.viewport(), |c, gl| {
            clear([0.0, 0.0, 0.0, 1.0], gl);
            match snapshot {
                Some(snapshot) => snapshot.layers.draw(&c, gl, &self.cfg),
                None => draw_waiting_spinner(&c, gl, &self.cfg, self.start_time.elapsed()),
            }
        });
    }
}

/// Draw a small dark-gray rotating arc at screen center as a "this client
/// is alive but hasn't received a snapshot yet" indicator.
fn draw_waiting_spinner(c: &Context, gl: &mut GlGraphics, cfg: &ClientConfig, elapsed: Duration) {
    use std::f64::consts::{PI, TAU};
    let cx = f64::from(cfg.x_resolution) / 2.0;
    let cy = f64::from(cfg.y_resolution) / 2.0;
    let radius = 20.0;
    let thickness = 2.0;
    // One revolution every 2 seconds.
    let phase = elapsed.as_secs_f64() * 0.5 * TAU;
    let arc = 1.5 * PI; // 270°
    let bounds = [cx - radius, cy - radius, radius * 2.0, radius * 2.0];
    CircleArc::new([0.25, 0.25, 0.25, 1.0], thickness, phase, phase + arc).draw(
        bounds,
        &c.draw_state,
        c.transform,
        gl,
    );
}

/// Spawn a thread to receive snapshots.
/// Inject them into the provided manager.
/// The thread runs until the run flag is tripped.
fn receive_snapshots(
    cfg: &ClientConfig,
    snapshot_manager: SnapshotManagerHandle,
    run_flag: RunFlag,
) {
    let mut subscriber =
        minusmq::pub_sub::Subscriber::new(&cfg.server_hostname, 6000, cfg.video_channel as u8);
    thread::Builder::new()
        .name("snapshot_receiver".to_string())
        .spawn(move || {
            loop {
                if !run_flag.should_run() {
                    info!("Snapshot receiver shutting down.");
                    break;
                }
                let buf = subscriber.recv();
                match rmp_serde::from_slice::<Snapshot>(&buf) {
                    Ok(msg) => {
                        *snapshot_manager.lock().unwrap() = Some(Arc::new(msg));
                    }
                    Err(e) => error!("receive error: {e}"),
                }
            }
        })
        .expect("Failed to spawn snapshot receiver thread");
}
