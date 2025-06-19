use crate::config::ClientConfig;
use crate::draw::Draw;
use anyhow::{anyhow, Result};
use graphics::clear;
use log::{error, info};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::sync::{Arc, Mutex};
use std::thread;
use tunnels_lib::RunFlag;
use tunnels_lib::Snapshot;
use zero_configure::pub_sub::Receiver;
use zmq::Context;

pub type SnapshotManagerHandle = Arc<Mutex<Option<SnapshotHandle>>>;
pub type SnapshotHandle = Arc<Snapshot>;

/// Top-level structure that owns all of the show data.
pub struct Show {
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManagerHandle,
    cfg: ClientConfig,
    run_flag: RunFlag,
    window: PistonWindow<Sdl2Window>,
}

impl Show {
    pub fn new(cfg: ClientConfig, ctx: Context, run_flag: RunFlag) -> Result<Self> {
        info!("Running on video channel {}.", cfg.video_channel);

        // Set up snapshot reception and management.
        let snapshot_manager = Arc::new(Mutex::new(None));
        receive_snapshots(&ctx, &cfg, snapshot_manager.clone(), run_flag.clone())?;

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
        // Note that with vsync enabled, this causes Piston to send incorrect
        // timesteps to update args; since we only use this for interpolating
        // timesync, it isn't a big deal.
        window.set_max_fps(120);

        Ok(Show {
            gl: GlGraphics::new(opengl),
            snapshot_manager,
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
                info!("Quit flag tripped, ending show.");
                break;
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
        let Some(snapshot) = self.snapshot_manager.lock().unwrap().clone() else {
            return;
        };

        self.gl.draw(args.viewport(), |c, gl| {
            // Clear the screen.
            clear([0.0, 0.0, 0.0, 1.0], gl);

            // Draw everything.
            snapshot.layers.draw(&c, gl, &self.cfg);
        });
    }
}

/// Spawn a thread to receive snapshots.
/// Inject them into the provided manager.
/// The thread runs until the run flag is tripped.
fn receive_snapshots(
    ctx: &Context,
    cfg: &ClientConfig,
    snapshot_manager: SnapshotManagerHandle,
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
        .spawn(move || loop {
            if !run_flag.should_run() {
                info!("Snapshot receiver shutting down.");
                break;
            }
            match receiver.receive_msg(true) {
                Ok(Some(msg)) => {
                    *snapshot_manager.lock().unwrap() = Some(Arc::new(msg));
                }
                Ok(None) => continue,
                Err(e) => error!("receive error: {e}"),
            }
        })?;
    Ok(())
}
