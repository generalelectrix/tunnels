use anyhow::{Context as _, Result, anyhow};
use client_lib::config::ClientConfig;
use graphics::{CircleArc, Context, clear};
use log::{error, info};
use opengl_graphics::{GlGraphics, OpenGL};
use piston_window::prelude::*;
use sdl2_window::Sdl2Window;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tunnelclient::draw::Draw;
use tunnels_model::mixer::VideoChannel;
use tunnels_model::show_frame::ShowFrame;
use tunnels_net::{FrameSubscriber, SubscriberStop};

/// A client's end of one console's stream of show frames.
///
/// Arriving frames land in a single slot, overwritten by every arrival: they
/// are published faster than they can be drawn, and a superseded frame is
/// never worth drawing.
pub struct FrameReceiver {
    /// The most recent frame to have arrived, if any has.
    latest: Arc<Mutex<Option<Arc<ShowFrame>>>>,
    /// Stops the subscription the receiving thread is reading from.
    stop: SubscriberStop,
    /// The thread taking frames off the stream, which lasts as long as the
    /// receiver it feeds. Empty once it has been taken out to be joined.
    service: Option<JoinHandle<()>>,
}

impl FrameReceiver {
    /// Subscribe to the frames a console publishes, and begin taking them off
    /// the stream.
    ///
    /// Receiving runs on a thread of its own until the subscription ends, which
    /// is what dropping the receiver brings about. A frame that cannot be
    /// decoded is logged and dropped; the stream is a sequence of independent
    /// frames, so losing one costs a frame of animation and nothing more.
    pub fn new(host: &str) -> Result<Self> {
        let mut subscriber = FrameSubscriber::new(host);
        let stop = subscriber.stop_handle();
        let latest: Arc<Mutex<Option<Arc<ShowFrame>>>> = Arc::new(Mutex::new(None));
        let service = thread::Builder::new()
            .name("frame_receiver".to_string())
            .spawn({
                let latest = latest.clone();
                move || {
                    let mut decode_errors = ErrorThrottle::new();
                    loop {
                        match subscriber.recv() {
                            None => {
                                info!("Frame subscription stopped.");
                                break;
                            }
                            Some(Ok(frame)) => *latest.lock().unwrap() = Some(Arc::new(frame)),
                            Some(Err(e)) => match decode_errors.record(Instant::now()) {
                                Some(ErrorReport::First) => error!("Frame decode error: {e}"),
                                Some(ErrorReport::Repeated(count)) => error!(
                                    "{count} frame decode errors in the last {}s, most recently: {e}",
                                    ERROR_REPORT_PERIOD.as_secs_f64(),
                                ),
                                None => (),
                            },
                        }
                    }
                }
            })
            .context("failed to spawn the frame receiver thread")?;
        Ok(Self {
            latest,
            stop,
            service: Some(service),
        })
    }

    /// The newest frame to have arrived, if any has.
    pub fn latest(&self) -> Option<Arc<ShowFrame>> {
        self.latest.lock().unwrap().clone()
    }
}

impl Drop for FrameReceiver {
    /// Stop the receiving thread and wait for it.
    ///
    /// The thread parks waiting for a frame, and a console that has stopped
    /// publishing sends no last frame to release it, so the subscription is
    /// stopped before the join: the wait is then only as long as it takes a
    /// released thread to return.
    fn drop(&mut self) {
        self.stop.stop();
        if let Some(service) = self.service.take() {
            let _ = service.join();
        }
    }
}

/// Top-level structure that owns all of the show data.
pub struct Show {
    gl: GlGraphics, // OpenGL drawing backend.
    frames: FrameReceiver,
    /// The video channel drawn out of every frame.
    video_channel: VideoChannel,
    cfg: ClientConfig,
    window: PistonWindow<Sdl2Window>,
    /// Reference instant for animating the waiting-for-frame spinner.
    start_time: Instant,
}

impl Show {
    pub fn new(cfg: ClientConfig) -> Result<Self> {
        let video_channel = VideoChannel(cfg.video_channel as usize);
        info!("Running on video channel {}.", cfg.video_channel);

        let frames = FrameReceiver::new(&cfg.server_hostname)?;

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
            window,
            start_time: Instant::now(),
        })
    }

    /// Run the show's event loop.
    pub fn run(&mut self) {
        // Run the event loop.
        while let Some(e) = self.window.next() {
            if let Some(r) = e.render_args() {
                self.render(&r);
            }
        }
    }

    /// Render a frame to the window.
    ///
    /// The latest show frame is expanded into geometry here rather than as it
    /// arrives, so the expansion happens once per drawn frame instead of once
    /// per published frame, of which there are more.
    ///
    /// Always clears to black, then either draws this client's video channel
    /// out of the latest frame or — until the first frame has arrived — a
    /// small spinner indicating the client is up and waiting. The
    /// unconditional clear is what keeps an unfed client from showing
    /// uninitialized GPU memory as static gray noise.
    ///
    /// The newest frame is drawn for as long as it is the newest, however long
    /// that turns out to be. A stream that stops leaves the last instant of
    /// the show standing on stage, which is a better thing to be looking at
    /// than a dark screen.
    fn render(&mut self, args: &RenderArgs) {
        let frame = self.frames.latest();
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

/// How long a run of failures goes unreported before its count is reported.
const ERROR_REPORT_PERIOD: Duration = Duration::from_secs(1);

/// A running count of a failure that repeats, reported at a bounded rate.
///
/// Frames arrive at 240 Hz, so a failure with a persistent cause — a stream
/// of garbage, a publisher that is not one — recurs as fast as they do.
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
    use std::sync::mpsc::channel;

    /// The interval show frames arrive at, and so the interval a failure with
    /// a persistent cause recurs at.
    const FRAME_INTERVAL: Duration = Duration::from_micros(4167);

    /// A dropped receiver takes its thread with it, whether or not frames are
    /// arriving.
    ///
    /// The thread parks waiting for a frame, and a console that is not
    /// publishing sends none to release it, so a drop that returns is proof
    /// that the wait ends from outside. Without that, quitting a client while
    /// the console was down would hang until the console came back.
    #[test]
    fn dropping_the_receiver_stops_its_thread() {
        let (dropped, completion) = channel();
        thread::spawn(move || {
            let receiver = FrameReceiver::new("127.0.0.1").unwrap();
            // Long enough for the thread to be waiting: for a frame if a
            // console happens to be publishing here, and for a console to
            // connect to if none is.
            thread::sleep(Duration::from_millis(200));
            drop(receiver);
            let _ = dropped.send(());
        });
        assert!(
            completion.recv_timeout(Duration::from_secs(10)).is_ok(),
            "dropping the receiver never finished: its thread is still waiting for a frame"
        );
    }

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
