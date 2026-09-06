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
use tunnels_model::show_frame::{FrameDecoder, ShowFrame};

/// The publish-subscribe channel show frames arrive on.
///
/// A frame describes the whole show, so every client subscribes to the same
/// channel and selects its own video channel out of the frame when it renders.
const FRAME_CHANNEL: u8 = 0;

/// The most recent show frame to have arrived, if any.
///
/// A single slot, overwritten by every arrival: frames are published faster
/// than they can be drawn, and a superseded frame is never worth drawing.
pub type FrameMailbox = Arc<Mutex<Option<ReceivedFrame>>>;

/// A show frame, and the moment it arrived.
pub struct ReceivedFrame {
    frame: Arc<ShowFrame>,
    received_at: Instant,
}

/// How old the newest frame may be before a client stops drawing it.
///
/// Frames arrive at 240 Hz, so a second of silence is 240 missed frames: the
/// stream is gone, not late. It is also longer than a reconnection takes when
/// there is anything to reconnect to — a subscriber retries immediately and
/// then backs off 100, 200 and 400 milliseconds — so a dropped connection to a
/// live console is restored well inside it and never reaches the screen.
const MAX_FRAME_AGE: Duration = Duration::from_secs(1);

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
    /// out of the latest frame or — if no frame has arrived recently enough to
    /// still describe the show — a small spinner indicating the client is up
    /// and waiting. The unconditional clear is what keeps an unfed client from
    /// showing uninitialized GPU memory as static gray noise.
    ///
    /// Drawing a stale frame forever is what a dead stream would otherwise look
    /// like: a screen indistinguishable from a working one, on a console that
    /// stopped publishing. The spinner is the difference between a failure the
    /// operator can see and one only the audience can.
    fn render(&mut self, args: &RenderArgs) {
        let frame = current_frame(&self.frames.lock().unwrap(), Instant::now());
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

/// The frame to draw, if the stream is still current enough to have one.
///
/// A frame older than the stream that produced it describes a show that has
/// moved on without this client, so it is not drawn at all.
fn current_frame(mailbox: &Option<ReceivedFrame>, now: Instant) -> Option<Arc<ShowFrame>> {
    mailbox.as_ref().and_then(|received| {
        (now.duration_since(received.received_at) < MAX_FRAME_AGE).then(|| received.frame.clone())
    })
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
            let mut decode_errors = ErrorThrottle::new();
            let mut decoder = FrameDecoder::new();
            loop {
                if !run_flag.should_run() {
                    info!("Frame receiver shutting down.");
                    break;
                }
                match decoder.decode(subscriber.recv()) {
                    Ok(frame) => {
                        *frames.lock().unwrap() = Some(ReceivedFrame {
                            frame: Arc::new(frame),
                            received_at: Instant::now(),
                        });
                    }
                    Err(e) => match decode_errors.record(Instant::now()) {
                        Some(ErrorReport::First) => error!("Frame decode error: {e}"),
                        Some(ErrorReport::Repeated(count)) => error!(
                            "{count} frame decode errors in the last {}s, most recently: {e}",
                            ERROR_REPORT_PERIOD.as_secs_f64(),
                        ),
                        None => (),
                    },
                }
            }
        })
        .expect("Failed to spawn frame receiver thread");
}

/// How long a run of failures goes unreported before its count is reported.
const ERROR_REPORT_PERIOD: Duration = Duration::from_secs(1);

/// A running count of a failure that repeats, reported at a bounded rate.
///
/// Frames arrive at 240 Hz, so a failure with a persistent cause — a peer
/// running a stale binary, a stream of garbage — recurs as fast as they do.
/// Reporting every occurrence buries the log under a flood that says nothing
/// the first line did not, so occurrences are counted between reports instead.
struct ErrorThrottle {
    /// Failures counted since the last report.
    unreported: u64,
    /// When the last report was made, if any has been.
    reported_at: Option<Instant>,
}

/// What a failure has to say for itself.
#[derive(Debug)]
enum ErrorReport {
    /// A failure that opens a run of them, worth reporting in full.
    First,
    /// How many failures a reporting period accumulated, this one included.
    Repeated(u64),
}

impl ErrorThrottle {
    fn new() -> Self {
        Self {
            unreported: 0,
            reported_at: None,
        }
    }

    /// Record a failure occurring at `now`, yielding what to report about it.
    ///
    /// The failure that opens a run is reported in full and those that follow
    /// it within a reporting period are only counted; the first failure past
    /// the period reports how many there have been. A failure arriving after a
    /// quiet stretch opens a fresh run rather than closing the last one.
    fn record(&mut self, now: Instant) -> Option<ErrorReport> {
        match self.reported_at {
            Some(reported_at) if now.duration_since(reported_at) < ERROR_REPORT_PERIOD => {
                self.unreported += 1;
                None
            }
            _ => {
                let counted = std::mem::take(&mut self.unreported);
                self.reported_at = Some(now);
                Some(if counted == 0 {
                    ErrorReport::First
                } else {
                    ErrorReport::Repeated(counted + 1)
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tunnels_model::show_frame::fixture;

    /// A frame is drawn while the stream is alive, and dropped once it is not.
    #[test]
    fn a_frame_outlives_its_stream_by_a_bounded_time() {
        let arrival = Instant::now();
        let mailbox = Some(ReceivedFrame {
            frame: Arc::new(fixture::default_frame()),
            received_at: arrival,
        });

        assert!(
            current_frame(&mailbox, arrival).is_some(),
            "a frame that just arrived is the frame to draw"
        );
        assert!(
            current_frame(&mailbox, arrival + MAX_FRAME_AGE - Duration::from_millis(1)).is_some(),
            "a frame is drawn for as long as it may be drawn"
        );
        assert!(
            current_frame(&mailbox, arrival + MAX_FRAME_AGE).is_none(),
            "a frame older than the limit is not drawn"
        );
        assert!(
            current_frame(&None, arrival).is_none(),
            "a client that has received nothing has nothing to draw"
        );
    }

    /// The interval show frames arrive at, and so the interval a failure with
    /// a persistent cause recurs at.
    const FRAME_INTERVAL: Duration = Duration::from_micros(4167);

    /// A failure that repeats every frame costs one log line per period.
    #[test]
    fn a_repeating_failure_is_counted_rather_than_reported() {
        let mut throttle = ErrorThrottle::new();
        let start = Instant::now();

        assert!(
            matches!(throttle.record(start), Some(ErrorReport::First)),
            "the failure that opens a run reports in full"
        );

        let mut elapsed = FRAME_INTERVAL;
        let mut counted = 0;
        while elapsed < ERROR_REPORT_PERIOD {
            assert!(
                throttle.record(start + elapsed).is_none(),
                "a failure {elapsed:?} into a run reported instead of counting"
            );
            counted += 1;
            elapsed += FRAME_INTERVAL;
        }

        match throttle.record(start + elapsed) {
            Some(ErrorReport::Repeated(count)) => assert_eq!(count, counted + 1),
            other => panic!("a period of failures reported {other:?}"),
        }

        assert!(
            matches!(
                throttle.record(start + elapsed + 3 * ERROR_REPORT_PERIOD),
                Some(ErrorReport::First)
            ),
            "a failure after a quiet stretch opens a fresh run"
        );
    }
}
