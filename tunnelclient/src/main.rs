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
extern crate zero_configure;

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
mod remote;

use config::load_config;
use std::env;
use zmq::Context;
use show::Show;
use utils::RunFlag;
use remote::run_remote;

fn main() {

    // Check if running in remote mode.
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'remote' to run in remote mode, \
         or the virtual video channel to listen to.");

    let mut ctx = Context::new();

    if first_arg == "remote" {
        run_remote(&mut ctx);
        return
    }

    let video_channel: u64 = first_arg.parse().expect("Video channel must be a positive integer.");

    let config_path = env::args().nth(2).expect("No config path arg provided.");

    let cfg = load_config(video_channel, config_path);

    let mut show = Show::new(cfg, &mut ctx, RunFlag::new());

    show.run();
}