// #![feature(rustc_macro)]

// #[macro_use]
// extern crate serde_derive;

extern crate piston;
extern crate interpolation;
extern crate graphics;
extern crate glutin_window;
extern crate sdl2_window;
extern crate opengl_graphics;
extern crate yaml_rust;
extern crate serde;
extern crate rmp_serde;
extern crate zmq;

mod constants {
    use std::f64::consts::PI;

    pub const TWOPI: f64 = 2.0 * PI;
}
mod config;
mod receive;
mod interpolate;
mod draw;


use config::{ClientConfig, config_from_command_line};
use graphics::clear;
use graphics::types::Color;
use opengl_graphics::{ GlGraphics, OpenGL };
use piston::window::WindowSettings;
use piston::event_loop::*;
use piston::input::*;
use receive::{Receiver, Snapshot};
use glutin_window::GlutinWindow as Window;
// use sdl2_window::Sdl2Window as Window;
use std::time::Instant;
use draw::Draw;

const BLACK: Color = [0.0, 0.0, 0.0, 1.0];


pub struct App {
    gl: GlGraphics, // OpenGL drawing backend.
    receiver: Receiver,
    most_recent_frame: Option<Snapshot>,
    config: ClientConfig
}

impl App {
    fn render(&mut self, args: &RenderArgs) {

        let maybe_frame = &self.most_recent_frame;
        let cfg = &self.config;

        self.gl.draw(args.viewport(), |c, gl| {

            // Clear the screen.
            clear(BLACK, gl);

            // Draw everything.
            if let Some(ref f) = *maybe_frame {
                f.draw(&c, gl, cfg);
            }
        });
    }

    fn update(&mut self, args: &UpdateArgs) {
        // Update newest frame is one is waiting.
        if let Some(f) = self.receiver.receive_newest() {
            match f {
                Ok(frame) => {
                    self.most_recent_frame = Some(frame);
                },
                Err(e) => {
                    println!("Error during frame receive: {:?}", e);
                }
            }
        }

    }
}




fn main() {

    let config = config_from_command_line();

    // Change this to OpenGL::V2_1 if not working.
    let opengl = OpenGL::V3_2;

    // Create an Glutin window.
    let mut window: Window = WindowSettings::new(
            "tunnelclient",
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
        receiver: Receiver::new("tcp://127.0.0.1:6000", &[]),
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