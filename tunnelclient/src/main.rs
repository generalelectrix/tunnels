// #![feature(rustc_macro)]

#[macro_use]
extern crate serde_derive;

extern crate piston;
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
mod sntp_service;
mod interpolate;
mod draw;
mod snapshot_manager;


use config::{ClientConfig, config_from_command_line};
use graphics::clear;
use graphics::types::Color;
use opengl_graphics::{ GlGraphics, OpenGL };
use piston_window::*;
use piston::window::{WindowSettings, AdvancedWindow};
use piston::event_loop::*;
use piston::input::*;
use receive::{Receive, SubReceiver, Snapshot};
use sntp_service::{synchronize, SntpSync};
use glutin_window::GlutinWindow;
use sdl2_window::Sdl2Window;
use std::time::{Duration, Instant};
use std::sync::mpsc::Receiver;
use std::thread::sleep;
use draw::Draw;
use zmq::Context;
use snapshot_manager::{SnapshotManager, SnapshotUpdateError};
use snapshot_manager::InterpResult::*;


pub struct App {
    gl: GlGraphics, // OpenGL drawing backend.
    snapshot_manager: SnapshotManager,
    sntp_sync: SntpSync,
    cfg: ClientConfig
}

impl App {
    fn render(&mut self, args: &RenderArgs) {

        // Get frame interpolation from the snapshot service.
        let delayed_time =
            self.sntp_sync.now_as_timestamp()
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

    fn update(&mut self, args: &UpdateArgs) {
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

    // Synchronize timing with master host.
    // Send 10 sync packets, half a second apart.
    let time_poll_period = Duration::from_millis(500);
    let n_time_calls: usize = 10;
    println!(
        "Synchronizing timing.  This will take about {} seconds.",
        (time_poll_period * n_time_calls as u32).as_secs());

    let sync = synchronize(
        &cfg.server_hostname, time_poll_period, n_time_calls);

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
        snapshot_manager: snapshot_manager,
        sntp_sync: sync,
        cfg: cfg
    };

    while let Some(e) = window.next() {

        if let Some(u) = e.update_args() {
            app.update(&u);
        }

        if let Some(r) = e.render_args() {
            app.render(&r);
        }
    }
}