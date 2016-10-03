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

mod config;
mod receive;
mod draw;

use piston::window::WindowSettings;
use piston::event_loop::*;
use piston::input::*;
//use glutin_window::GlutinWindow as Window;
use sdl2_window::Sdl2Window as Window;

use std::f64::consts::PI;
use std::time::Instant;

use opengl_graphics::{ GlGraphics, OpenGL };

use receive::{Receiver, Snapshot};

use config::{ClientConfig, config_from_command_line};

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

            // Clear the screen.
            clear(BLACK, gl);

            /*
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