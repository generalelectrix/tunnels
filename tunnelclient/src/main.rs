#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate simple_error;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate derive_more;
extern crate glutin_window;
extern crate graphics;
extern crate hostname;
extern crate interpolation;
extern crate opengl_graphics;
extern crate piston_window;
extern crate regex;
extern crate rmp_serde;
extern crate sdl2_window;
extern crate serde;
extern crate stats;
extern crate yaml_rust;
extern crate zero_configure;
extern crate zmq;

mod constants {
    use std::f64::consts::PI;
    pub const TWOPI: f64 = 2.0 * PI;
}

mod config;
mod draw;
mod interpolate;
mod receive;
mod remote;
mod show;
mod snapshot_manager;
mod timesync;
mod utils;

use config::ClientConfig;
use remote::{administrate, run_remote};
use show::Show;
use std::env;
use utils::RunFlag;
use zmq::Context;

fn main() {
    // Check if running in remote mode.
    let first_arg = env::args().nth(1).expect(
        "First argument must be 'remote' to run in remote mode, \
        'admin' to run the client administrator,
         or the integer virtual video channel to listen to.",
    );

    let mut ctx = Context::new();

    if first_arg == "remote" {
        run_remote(&mut ctx);
    } else if first_arg == "admin" {
        //        let admin = Administrator::new();
        //
        //        ::std::thread::sleep_ms(2000);
        //
        //        let clients = admin.clients();
        //        println!("Clients: {:?}", clients);
        //
        //        let config = ClientConfig::load(0, "cfg/monitor.yaml");
        //        match admin.run_with_config(&clients[0], config.unwrap()) {
        //            Ok(msg) => println!("Success:\n{}", msg),
        //            Err(e) => println!("Error:\n{:?}", e),
        //        }
        //        return
        administrate();
    } else {
        let video_channel: u64 = first_arg
            .parse()
            .expect("Video channel must be a positive integer.");

        let config_path = env::args().nth(2).expect("No config path arg provided.");

        let cfg = ClientConfig::load(video_channel, &config_path).expect("Failed to load config");

        let mut show = Show::new(cfg, &mut ctx, RunFlag::new()).expect("Failed to initialize show");

        show.run();
    }
}
