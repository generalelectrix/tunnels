// #![feature(rustc_macro)]

// #[macro_use]
// extern crate serde_derive;

extern crate piston;
extern crate graphics;
extern crate glutin_window;
extern crate sdl2_window;
extern crate opengl_graphics;
extern crate yaml_rust;
extern crate serde;
extern crate rmp_serde;
extern crate zmq;

mod receive;

use yaml_rust::YamlLoader;

use piston::window::WindowSettings;
use piston::event_loop::*;
use piston::input::*;
//use glutin_window::GlutinWindow as Window;
use sdl2_window::Sdl2Window as Window;

use std::f64::consts::PI;
use std::fs::File;
use std::io::Read;
use std::time::Instant;

use opengl_graphics::{ GlGraphics, OpenGL };

use receive::{Receiver, Snapshot};

const TWOPI: f64 = 2.0 * PI;

pub struct App {
    gl: GlGraphics, // OpenGL drawing backend.
    receiver: Receiver,
    most_recent_frame: Option<Snapshot>,
    config: ClientConfig
}

impl App {
    fn render(&mut self, args: &RenderArgs) {
        use graphics::*;

        const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
        const RED:   [f32; 4] = [1.0, 0.0, 0.0, 1.0];
        const BLUE:  [f32; 4] = [0.0, 0.0, 1.0, 1.0];
        const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

        let bound = rectangle::centered([0.0, 0.0, 550.0, 340.0]);
        let (x, y) = ((args.width / 2) as f64,
                      (args.height / 2) as f64);

        self.gl.draw(args.viewport(), |c, gl| {
            /*
            // Clear the screen.
            clear(BLACK, gl);

            let transform = c.transform.trans(x, y)
                                       .rot_rad(rotation);

            for seg in 0..128 {
                circle_arc(WHITE, 20.0, start, end, bound, transform, gl);
            }
            */

        });
    }

    fn update(&mut self, args: &UpdateArgs) {
        // block until a new frame is available
        // this is completely wrong but fine for testing
        self.most_recent_frame = Some(self.receiver.receive());
    }
}

struct ClientConfig {
    x_resolution: u32,
    y_resolution: u32,
    anti_alias: bool,
    fullscreen: bool,
    critical_size: u64,
    thickness_scale: f64,
    x_center: u64,
    y_center: u64
}

/// Parses first command line arg as path to a yaml config file.
/// Loads, parses, and returns the config.
/// Panics if something goes wrong.
fn config_from_command_line() -> ClientConfig {
    let config_path = std::env::args().nth(1).expect("No config path arg provided.");
    let mut config_file = File::open(config_path).unwrap();
    let mut config_file_string = String::new();
    config_file.read_to_string(&mut config_file_string).unwrap();
    let docs = YamlLoader::load_from_str(&config_file_string).unwrap();
    let cfg = &docs[0];
    let x_resolution = cfg["x_resolution"].as_i64().unwrap() as u32;
    let y_resolution = cfg["y_resolution"].as_i64().unwrap() as u32;
    ClientConfig {
        x_resolution: x_resolution,
        y_resolution: y_resolution,
        anti_alias: cfg["anti_alias"].as_bool().unwrap(),
        fullscreen: cfg["fullscreen"].as_bool().unwrap(),
        critical_size: std::cmp::min(x_resolution, y_resolution) as u64,
        thickness_scale: 0.5,
        x_center: (x_resolution / 2) as u64,
        y_center: (y_resolution / 2) as u64
    }
}


fn main() {

    let config = config_from_command_line();

    // Change this to OpenGL::V2_1 if not working.
    let opengl = OpenGL::V3_2;

    // Create an Glutin window.
    let mut window: Window = WindowSettings::new(
            "spinning-square",
            [config.x_resolution, config.y_resolution]
        )
        .opengl(opengl)
        .exit_on_esc(true)
        .vsync(true)
        .samples(if config.anti_alias {4} else {0})
        .fullscreen(config.fullscreen)
        .build()
        .unwrap();

    // Create a new game and run it.
    let mut app = App {
        gl: GlGraphics::new(opengl),
        receiver: Receiver::new("tcp://localhost:6000"),
        most_recent_frame: None,
        config: config
    };

    let mut events = window.events();
    while let Some(e) = events.next(&mut window) {

        if let Some(u) = e.update_args() {
            app.update(&u);
        }

        if let Some(r) = e.render_args() {
            app.render(&r);
        }
    }
}