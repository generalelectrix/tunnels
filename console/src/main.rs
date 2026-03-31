use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use console::bootstrap_controller::BootstrapController;
use log::error;
use midi_harness::install_midi_device_change_handler;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};

use tunnels::control::CommandClient;
use tunnels::gui_state::GuiState;
use tunnels::midi::{default_midi_slots, ControlEventHandler};
use tunnels::show::Show;

/// Approximately 240 fps.
const RENDER_INTERVAL: Duration = Duration::from_nanos(16666667 / 4);

fn main() -> Result<()> {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())?;

    let (send_control_event, recv_control_event) = channel();
    install_midi_device_change_handler(ControlEventHandler(send_control_event.clone()))?;

    let client = CommandClient::new(send_control_event.clone());
    let gui_state = Arc::new(GuiState::default());
    let show_gui_state = gui_state.clone();

    // Show worker thread — starts with empty config, GUI sends MetaCommands.
    std::thread::spawn(move || {
        let show = Show::new(
            default_midi_slots(),
            vec![],
            send_control_event,
            recv_control_event,
            None,
            false,
            None,
            Some(show_gui_state),
        );
        match show {
            Ok(mut show) => {
                if let Err(e) = show.run(RENDER_INTERVAL) {
                    error!("Show error: {e:#}");
                }
            }
            Err(e) => {
                error!("Failed to create show: {e:#}");
            }
        }
    });

    let admin: Arc<dyn console::admin_panel::AdminService> =
        Arc::new(BootstrapController::new(Some(Duration::from_secs(10))));

    let hostname = hostname::get()
        .map(|h| h.into_string().unwrap_or_else(|_| "unknown".to_string()))
        .unwrap_or_else(|_| "unknown".to_string());

    console::run_config_gui(client, gui_state, admin, hostname)
}
