//! Controls for pushing the bundled TouchOSC layout to a device.

use eframe::egui;

/// Render the sync control, and while the server is running the modal that
/// holds the transfer in front of the user until they stop it.
pub fn touchosc_server_ui(ui: &mut egui::Ui, running: bool) -> Option<TouchOscServerAction> {
    let mut action = None;

    if ui.button("Send Template To Device").clicked() {
        action = Some(TouchOscServerAction::Start);
    }

    if running {
        egui::Modal::new(egui::Id::new("touchosc_sync_modal")).show(ui.ctx(), |ui| {
            ui.set_width(350.0);
            ui.heading("TouchOSC Sync");
            ui.add_space(8.0);
            ui.label(
                "Open TouchOSC Mk1 on your device, open Layout \u{2192} Add, \
                 and select this computer to sync the template.",
            );
            ui.add_space(12.0);
            if ui.button("Stop").clicked() {
                action = Some(TouchOscServerAction::Stop);
            }
        });
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

    /// The idle control starts a transfer; the modal that replaces it stops
    /// one.
    #[test]
    fn each_state_offers_the_action_that_changes_it() {
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
