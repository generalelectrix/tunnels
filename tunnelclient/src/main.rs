extern crate piston;
extern crate graphics;
extern crate glutin_window;
extern crate sdl2_window;
extern crate opengl_graphics;
extern crate yaml_rust;
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

const TWOPI: f64 = 2.0 * PI;

pub struct App {
    gl: GlGraphics, // OpenGL drawing backend.
    rotation: f64,   // Rotation for the square.
    marquee: f64    // marquee rotation position
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
        let rotation = self.rotation;
        let marquee = self.marquee;
        let (x, y) = ((args.width / 2) as f64,
                      (args.height / 2) as f64);

        let extrapolation = 0.3 * args.ext_dt;
        println!("{}", args.ext_dt);

        self.gl.draw(args.viewport(), |c, gl| {
            // Clear the screen.
            clear(BLACK, gl);

            let transform = c.transform.trans(x, y)
                                       .rot_rad(rotation);

            //circle_arc(RED, 20.0, 0.0, 3.141, bound, transform, gl);
            let seg_width = TWOPI / 128.0;
            for seg in 0..128 {
                if seg % 2 == 0 {
                    let start = (seg as f64 * seg_width) + marquee + extrapolation;
                    let end = start + seg_width;
                    circle_arc(WHITE, 20.0, start, end, bound, transform, gl);
                }
            }

        });
    }

    fn update(&mut self, args: &UpdateArgs) {
        // Rotate 2 radians per second.
        self.rotation = (self.rotation + 0.0 * args.dt) % TWOPI;
        self.marquee = (self.marquee + 0.3 * args.dt) % TWOPI;
    }
}

struct ClientConfig {
    x_resolution: u32,
    y_resolution: u32,
    anti_alias: bool,
    fullscreen: bool
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
    ClientConfig {
        x_resolution: cfg["x_resolution"].as_i64().unwrap() as u32,
        y_resolution: cfg["y_resolution"].as_i64().unwrap() as u32,
        anti_alias: cfg["anti_alias"].as_bool().unwrap(),
        fullscreen: cfg["fullscreen"].as_bool().unwrap(),
    }
}

trait Draw {
    fn draw(&self, gl: GlGraphics);
}

trait Interpolate {
    fn interpolate_with(&self, other: &Self, time: &Instant) -> Self;
}

#[derive(Clone, Debug)]
pub struct Arc {
    level: u64,
    thickness: f32,
    hue: f32,
    sat: f32,
    val: u64,
    x: f32,
    y: f32,
    rad_x: f32,
    rad_y: f32,
    start: f32,
    stop: f32,
    rot_angle: f32
}

impl Interpolate for Arc {
    fn interpolate_with(&self, other: &Self, time: &Instant) -> Self {
        other.clone()
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
        rotation: 0.0,
        marquee: 0.0
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