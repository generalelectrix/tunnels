use eframe::egui;

use crate::STATUS_COLORS;

/// Renders clock service start/stop controls.
pub fn clock_service_ui(ui: &mut egui::Ui, running: bool) -> Option<ClockServiceAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.strong("Clock Service");
        ui.add_space(8.0);

        let (status_label, status_color) = if running {
            ("Running", STATUS_COLORS.active)
        } else {
            ("Stopped", STATUS_COLORS.inactive)
        };
        ui.colored_label(status_color, status_label);
        ui.add_space(8.0);

        let button_label = if running { "Stop" } else { "Start" };
        if ui.button(button_label).clicked() {
            action = Some(if running {
                ClockServiceAction::Stop
            } else {
                ClockServiceAction::Start
            });
        }
    });

    action
}

pub enum ClockServiceAction {
    Start,
    Stop,
}
