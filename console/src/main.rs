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

#[cfg(target_os = "macos")]
fn init_logger() {
    oslog::OsLogger::new("com.generalelectrix.tunnels")
        .level_filter(log::LevelFilter::Info)
        .init()
        .expect("failed to initialize os_log");
}

#[cfg(not(target_os = "macos"))]
fn init_logger() {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default())
        .expect("failed to initialize logger");
}

fn main() -> Result<()> {
    init_logger();

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

    // Moved into the creator closure.
    let mut startup = Some((send_control_event, recv_control_event, client, admin, hostname));

    eframe::run_native(
        "Tunnels",
        console::native_options(),
        Box::new(move |cc| {
            stage_theme::apply(&cc.egui_ctx);

            let (send, recv, client, admin, hostname) =
                startup.take().expect("creator closure called once");

            let repaint: RepaintSignal = {
                let ctx = cc.egui_ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };

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
                client, gui_state, admin, hostname, repaint, envelope_rx,
            )))
        }),
    )
    .unwrap();
    Ok(())
}
