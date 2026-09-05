use anyhow::{Result, anyhow};
use client_lib::config::ClientConfig;
use graphics::{CircleArc, Context, clear};
use log::{error, info};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tunnelclient::draw::Draw;
use tunnels_lib::RunFlag;
use tunnels_model::mixer::{Mixer, VideoChannel};
use tunnels_model::show_frame::ShowFrame;

/// The publish-subscribe channel show frames arrive on.
///
/// A frame describes the whole show, so every client subscribes to the same
/// channel and selects its own video channel out of the frame when it renders.
const FRAME_CHANNEL: u8 = 0;

/// The most recent show frame to have arrived, if any.
///
/// A single slot, overwritten by every arrival: frames are published faster
/// than they can be drawn, and a superseded frame is never worth drawing.
pub type FrameMailbox = Arc<Mutex<Option<Arc<ShowFrame>>>>;

/// Top-level structure that owns all of the show data.
pub struct Show {
    gl: GlGraphics, // OpenGL drawing backend.
    frames: FrameMailbox,
    /// The video channel drawn out of every frame.
    video_channel: VideoChannel,
    cfg: ClientConfig,
    run_flag: RunFlag,
    window: PistonWindow<Sdl2Window>,
    /// Reference instant for animating the waiting-for-frame spinner.
    start_time: Instant,
}

impl Show {
    pub fn new(cfg: ClientConfig, run_flag: RunFlag) -> Result<Self> {
        let video_channel = video_channel(&cfg)?;
        info!("Running on video channel {}.", cfg.video_channel);

        // Set up frame reception and management.
        let frames = Arc::new(Mutex::new(None));
        receive_frames(&cfg, frames.clone(), run_flag.clone());

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
        window.set_max_fps(120);

        Ok(Show {
            gl: GlGraphics::new(opengl),
            frames,
            video_channel,
            cfg,
            run_flag,
            window,
            start_time: Instant::now(),
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

        self.run_flag.stop();
    }

    /// Render a frame to the window.
    ///
    /// The latest show frame is expanded into geometry here rather than as it
    /// arrives, so the expansion happens once per drawn frame instead of once
    /// per published frame, of which there are more.
    ///
    /// Always clears to black, then either draws this client's video channel
    /// out of the latest frame or — if no frame has arrived yet — a small
    /// spinner indicating the client is up and waiting. The unconditional clear
    /// is what keeps an unfed client from showing uninitialized GPU memory as
    /// static gray noise.
    fn render(&mut self, args: &RenderArgs) {
        let frame = self.frames.lock().unwrap().clone();
        let layers = frame.as_ref().map(|frame| {
            frame
                .mixer
                .render_video_channel(self.video_channel, frame.render_context())
        });
        self.gl.draw(args.viewport(), |c, gl| {
            clear([0.0, 0.0, 0.0, 1.0], gl);
            match &layers {
                Some(layers) => layers.draw(&c, gl, &self.cfg),
                None => draw_waiting_spinner(&c, gl, &self.cfg, self.start_time.elapsed()),
            }
        });
    }
}

/// The video channel a configuration selects.
///
/// A configuration names a channel that may not exist, and a channel a mixer
/// does not have would silently draw nothing at all, so it is rejected here
/// instead.
fn video_channel(cfg: &ClientConfig) -> Result<VideoChannel> {
    usize::try_from(cfg.video_channel)
        .ok()
        .filter(|channel| *channel < Mixer::N_VIDEO_CHANNELS)
        .map(VideoChannel)
        .ok_or_else(|| {
            anyhow!(
                "video channel {} does not exist; a show has {} video channels, numbered from 0",
                cfg.video_channel,
                Mixer::N_VIDEO_CHANNELS,
            )
        })
}

/// Draw a small dark-gray rotating arc at screen center as a "this client
/// is alive but hasn't received a frame yet" indicator.
fn draw_waiting_spinner(c: &Context, gl: &mut GlGraphics, cfg: &ClientConfig, elapsed: Duration) {
    use std::f64::consts::{PI, TAU};
    let cx = f64::from(cfg.x_resolution) / 2.0;
    let cy = f64::from(cfg.y_resolution) / 2.0;
    let radius = 20.0;
    let thickness = 2.0;
    // One revolution every 2 seconds.
    let phase = elapsed.as_secs_f64() * 0.5 * TAU;
    let arc = 1.5 * PI; // 270°
    let bounds = [cx - radius, cy - radius, radius * 2.0, radius * 2.0];
    CircleArc::new([0.25, 0.25, 0.25, 1.0], thickness, phase, phase + arc).draw(
        bounds,
        &c.draw_state,
        c.transform,
        gl,
    );
}

/// Spawn a thread to receive show frames.
/// Inject them into the provided mailbox.
/// The thread runs until the run flag is tripped.
///
/// A frame that cannot be decoded is logged and dropped; the stream is a
/// sequence of independent frames, so losing one costs a frame of animation
/// and nothing more.
fn receive_frames(cfg: &ClientConfig, frames: FrameMailbox, run_flag: RunFlag) {
    let mut subscriber =
        minusmq::pub_sub::Subscriber::new(&cfg.server_hostname, 6000, FRAME_CHANNEL);
    thread::Builder::new()
        .name("frame_receiver".to_string())
        .spawn(move || {
            loop {
                if !run_flag.should_run() {
                    info!("Frame receiver shutting down.");
                    break;
                }
                let buf = subscriber.recv();
                match ShowFrame::decode(&buf) {
                    Ok(frame) => {
                        *frames.lock().unwrap() = Some(Arc::new(frame));
                    }
                    Err(e) => error!("receive error: {e}"),
                }
            }
        })
        .expect("Failed to spawn frame receiver thread");
}
