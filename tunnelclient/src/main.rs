// #![feature(rustc_macro)]

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate simple_error;

extern crate piston_window;
extern crate interpolation;
extern crate graphics;
extern crate glutin_window;
extern crate sdl2_window;
extern crate opengl_graphics;
extern crate yaml_rust;
extern crate serde;
extern crate rmp_serde;
extern crate zmq;
extern crate stats;

mod constants {
    use std::f64::consts::PI;

    pub const TWOPI: f64 = 2.0 * PI;
}
mod utils;
mod config;
mod receive;
mod timesync;
mod interpolate;
mod draw;
mod snapshot_manager;


use config::{ClientConfig, config_from_command_line};
use graphics::clear;
use opengl_graphics::{ GlGraphics, OpenGL };
use piston_window::*;
use receive::{SubReceiver, Snapshot};
use timesync::{Client as TimesyncClient, Timesync};
use glutin_window::GlutinWindow;
use sdl2_window::Sdl2Window;
use std::time::Duration;
use std::sync::mpsc::Receiver;
use std::thread::sleep;
use draw::Draw;
use zmq::Context;
use snapshot_manager::{SnapshotManager, SnapshotUpdateError};
use snapshot_manager::InterpResult::*;


pub struct App {
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManager,
    timesync: Timesync,
    cfg: ClientConfig
}

impl App {
    fn render(&mut self, args: &RenderArgs) {

        // Get frame interpolation from the snapshot service.
        let delayed_time =
            self.timesync.now_as_timestamp()
            - self.cfg.render_delay as f64;

        println!("Delayed time: {}", delayed_time);
        println!("Latest snapshot: {}", self.snapshot_manager.latest_time());

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

    fn update(&mut self) {
        // Update the state of the snapshot manager.
        let update_result = self.snapshot_manager.update();
        if let Err(e) = update_result {
            let msg = match e {
                SnapshotUpdateError::Disconnected => "disconnected"
            };
            println!("An error occurred during snapshot update: {:?}", msg);
        }
    }
}

fn main() {

    let cfg = config_from_command_line();

    // Change this to OpenGL::V2_1 if not working.
    let opengl = OpenGL::V3_2;

    // Create an Glutin window.
    let mut window: PistonWindow<Sdl2Window> = WindowSettings::new(
            "tunnelclient",
            [cfg.x_resolution, cfg.y_resolution]
        )
        .opengl(opengl)
        .exit_on_esc(true)
        .vsync(true)
        .samples(if cfg.anti_alias {4} else {0})
        .fullscreen(cfg.fullscreen)
        .build()
        .unwrap();

    window.set_capture_cursor(true);
    window.set_max_fps(120);

    // Create zmq context.
    let mut ctx = Context::new();

    // Start up the timesync service.
    let mut timesync_client = TimesyncClient::new(&cfg.server_hostname, &mut ctx);

    // Synchronize timing with master host.
    println!(
        "Synchronizing timing.  This will take about {} seconds.",
        (timesync_client.poll_period * timesync_client.n_meas as u32).as_secs());

    let sync = timesync_client.synchronize().unwrap();

    // Set up snapshot reception and management.
    let snapshot_queue: Receiver<Snapshot> =
        SubReceiver::new(&cfg.server_hostname, 6000, cfg.video_channel.as_bytes(), &mut ctx)
        .run_async();

    let snapshot_manager = SnapshotManager::new(snapshot_queue);

    // sleep for a render delay to make sure we have snapshots before we start rendering
    sleep(Duration::from_millis(cfg.render_delay));

    // Create a new game and run it.
    let mut app = App {
        gl: GlGraphics::new(opengl),
        snapshot_manager,
        timesync: sync,
        cfg
    };

    while let Some(e) = window.next() {

        if let Some(_) = e.update_args() {
            app.update();
        }

        if let Some(r) = e.render_args() {
            app.render(&r);
        }
    }
}