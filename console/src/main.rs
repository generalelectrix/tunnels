use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::Duration;

use anyhow::Result;
use console::bootstrap_controller::BootstrapController;
use log::error;
use midi_harness::install_midi_device_change_handler;

use tunnels::control::CommandClient;
use tunnels::gui_state::GuiState;
use tunnels::midi::ControlEventHandler;
use tunnels::show::Show;
use tunnels_lib::repaint::RepaintSignal;

/// Approximately 240 fps.
const RENDER_INTERVAL: Duration = Duration::from_nanos(16666667 / 4);

/// Backlog of in-flight log records between the producer and the drain thread.
/// Records are dropped (and counted) when this fills, so logging never blocks
/// a real-time thread.
const LOG_CHANNEL_CAPACITY: usize = 1024;

/// Per-severity scrollback retained for the in-GUI log view.
const LOG_SCROLLBACK_PER_SEVERITY: usize = 500;

/// Override NSApplication's terminate: to send performClose: to the key
/// window instead of killing the process. This converts Cmd+Q into the
/// same close event as clicking the red window button, which our
/// CloseHandler can intercept with a confirmation dialog.
#[cfg(target_os = "macos")]
fn install_terminate_override() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::sel;

    unsafe extern "C" fn terminate_override(
        _this: *mut AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) {
        use objc2_app_kit::NSApplication;
        use objc2_foundation::MainThreadMarker;

        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let app = NSApplication::sharedApplication(mtm);
        if let Some(window) = app.keyWindow() {
            unsafe { window.performClose(None) };
        }
    }

    unsafe {
        let class = AnyClass::get("NSApplication").expect("NSApplication class not found");
        let method = class
            .instance_method(sel!(terminate:))
            .expect("terminate: method not found");
        let imp: Imp = std::mem::transmute(terminate_override as *mut ());
        method.set_implementation(imp);
    }
}

fn main() -> Result<()> {
    // The application logs at Warn. Capture records into the in-GUI status view
    // (Warn floor, matching the stderr logger) in addition to stderr. On macOS,
    // Console.app still captures stderr from SimpleLogger under the `.app` route,
    // so dropping oslog loses nothing.
    let (capture, log_rx) =
        gui_common::log_status::channel(LOG_CHANNEL_CAPACITY, simplelog::LevelFilter::Warn);
    simplelog::CombinedLogger::init(vec![
        simplelog::SimpleLogger::new(simplelog::LevelFilter::Warn, simplelog::Config::default()),
        Box::new(capture),
    ])
    .expect("failed to initialize logger");

    #[cfg(target_os = "macos")]
    install_terminate_override();

    let (send_control_event, recv_control_event) = channel();
    install_midi_device_change_handler(ControlEventHandler(send_control_event.clone()))?;

    let client = CommandClient::new(send_control_event.clone());

    let admin: Arc<dyn console::admin_panel::AdminService> =
        Arc::new(BootstrapController::new(Some(Duration::from_secs(10))));

    let hostname = hostname::get()
        .map(|h| h.into_string().unwrap_or_else(|_| "unknown".to_string()))
        .unwrap_or_else(|_| "unknown".to_string());

    // eframe's creator closure is FnMut; `Option::take` is the idiomatic way
    // to hand ownership of move-only setup values into a FnOnce-shaped body.
    let mut startup = Some((
        send_control_event,
        recv_control_event,
        client,
        admin,
        hostname,
        log_rx,
    ));

    eframe::run_native(
        "Tunnels",
        console::native_options(),
        Box::new(move |cc| {
            stage_theme::apply(&cc.egui_ctx);

            let (send, recv, client, admin, hostname, log_rx) =
                startup.take().expect("creator closure called once");

            let repaint: RepaintSignal = {
                let ctx = cc.egui_ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };

            // Build the in-GUI log surfaces and spawn the drain thread that
            // moves captured records into scrollback and fires the repaint.
            let log_alert = Arc::new(gui_common::log_status::LogAlert::new(repaint.clone()));
            let scrollback = Arc::new(std::sync::Mutex::new(
                gui_common::log_status::Scrollback::new(LOG_SCROLLBACK_PER_SEVERITY),
            ));
            let viewing = Arc::new(std::sync::atomic::AtomicBool::new(false));
            gui_common::log_status::spawn_drain_thread(
                log_rx,
                scrollback.clone(),
                log_alert.clone(),
                viewing.clone(),
            );
            let log_status =
                gui_common::log_status::LogStatusState::new(log_alert, scrollback, viewing);

            let gui_state = Arc::new(GuiState::new(repaint.clone()));
            let show_gui_state = gui_state.clone();

            let (envelope_tx, envelope_rx) = channel();

            std::thread::spawn(move || {
                let mut show = Show::new(send, recv, show_gui_state, envelope_tx)
                    .expect("show construction should not fail at startup");
                loop {
                    if let Err(e) = show.run(RENDER_INTERVAL) {
                        error!("Show error: {e:#} — restarting show loop");
                    }
                }
            });

            Ok(Box::new(console::ConfigApp::new(
                client,
                gui_state,
                admin,
                hostname,
                repaint,
                envelope_rx,
                log_status,
            )))
        }),
    )
    .unwrap();
    Ok(())
}
