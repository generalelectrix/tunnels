//! Controls for pushing the bundled TouchOSC layout to a device.

use eframe::egui;
use gui_common::STATUS_COLORS;

/// Render the layout server's status and its start/stop control, returning the
/// action the user asked for.
pub fn touchosc_server_ui(ui: &mut egui::Ui, running: bool) -> Option<TouchOscServerAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.strong("TouchOSC Sync");
        ui.add_space(8.0);

        let (status_label, status_color) = if running {
            ("Serving", STATUS_COLORS.active)
        } else {
            ("Idle", STATUS_COLORS.inactive)
        };
        ui.colored_label(status_color, status_label);
        ui.add_space(8.0);

        if running {
            if ui.button("Stop").clicked() {
                action = Some(TouchOscServerAction::Stop);
            }
        } else if ui.button("Send Template To Device").clicked() {
            action = Some(TouchOscServerAction::Start);
        }
    });

    if running {
        ui.label(
            "Open TouchOSC Mk1 on your device, open Layout \u{2192} Add, and \
             select this computer to sync the template. Stop the server when \
             the transfer is done.",
        );
    }

    action
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchOscServerAction {
    Start,
    Stop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;
    use std::cell::Cell;

    /// The control offers the action that changes the server's state, and
    /// reports it when pressed.
    #[test]
    fn button_reports_the_action_for_each_state() {
        for (running, label, expected) in [
            (
                false,
                "Send Template To Device",
                TouchOscServerAction::Start,
            ),
            (true, "Stop", TouchOscServerAction::Stop),
        ] {
            let action = Cell::new(None);
            let mut harness = Harness::new_ui(|ui| {
                if let Some(a) = touchosc_server_ui(ui, running) {
                    action.set(Some(a));
                }
            });
            harness.run();
            harness.get_by_label(label).click();
            harness.run();
            assert_eq!(action.get(), Some(expected), "with running = {running}");
        }
    }
}
