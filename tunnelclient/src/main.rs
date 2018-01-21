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
mod show;

use config::config_from_command_line;
use std::thread;
use std::sync::atomic::{ATOMIC_BOOL_INIT};
use zmq::Context;
use show::Show;
use utils::RunFlag;

fn main() {
    let cfg = config_from_command_line();

    let mut ctx = Context::new();

    let mut show = Show::new(cfg, &mut ctx, RunFlag::new());

    show.run();
}