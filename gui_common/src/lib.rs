pub mod background_task;
pub mod midi_panel;

/// Ability to show messages to the user.
///
/// Implemented by the app type so infrastructure code (background tasks,
/// error handlers, etc.) can show notifications without knowing about
/// the specific UI implementation.
pub trait UserNotify {
    fn notify(&mut self, title: &str, message: &str);
    fn notify_error(&mut self, error: anyhow::Error);
}

use eframe::egui::{self, Color32};

/// Semantic status colors for consistent theming across panels.
pub struct StatusColors {
    /// Neutral/unassigned state.
    pub inactive: Color32,
    /// Connected/running state.
    pub active: Color32,
    /// Degraded/attention-needed state.
    pub warning: Color32,
    /// Disconnected/failed state.
    pub error: Color32,
    /// Inline validation error text.
    pub error_text: Color32,
    /// Fill color for confirm/accept/apply buttons.
    pub confirm_button: Color32,
    /// Fill color for cancel/revert/dismiss buttons.
    pub cancel_button: Color32,
}

pub const STATUS_COLORS: StatusColors = StatusColors {
    inactive: Color32::GRAY,
    active: Color32::GREEN,
    warning: Color32::from_rgb(255, 165, 0),
    error: Color32::from_rgb(255, 80, 80),
    error_text: Color32::RED,
    confirm_button: Color32::from_rgb(30, 100, 50),
    cancel_button: Color32::from_rgb(80, 80, 80),
};

/// A confirm/accept/apply button with semantic styling.
pub fn confirm_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(text).fill(STATUS_COLORS.confirm_button))
        .clicked()
}

/// A confirm button that can be disabled.
pub fn confirm_button_enabled(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(text).fill(STATUS_COLORS.confirm_button),
    )
    .clicked()
}

/// A cancel/revert/dismiss button with semantic styling.
pub fn cancel_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(text).fill(STATUS_COLORS.cancel_button))
        .clicked()
}

/// Result of a drag-and-drop reorder interaction on a single row.
pub struct DndReorderResult {
    /// If `Some`, this row received a drop — the caller should swap source and target.
    pub swap: Option<(usize, usize)>,
}

/// Apply drag-and-drop reorder behavior to a response.
///
/// Handles cursor feedback (grab/grabbing icons), drag payload management,
/// and painting a drop-indicator line. Call this once per row in a reorderable
/// list, then collect the `swap` results to apply after the loop.
pub fn dnd_reorder(
    ui: &egui::Ui,
    response: &egui::Response,
    row_index: usize,
    indicator_x_range: impl Into<egui::Rangef>,
) -> DndReorderResult {
    if response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    response.dnd_set_drag_payload(row_index);

    let mut swap = None;
    if let Some(source_idx) = response.dnd_release_payload::<usize>() {
        swap = Some((*source_idx, row_index));
    }

    if let Some(source_idx) = response.dnd_hover_payload::<usize>() {
        let selection_color = ui.style().visuals.selection.bg_fill;
        let y = if *source_idx <= row_index {
            response.rect.bottom()
        } else {
            response.rect.top()
        };
        ui.painter().hline(
            indicator_x_range,
            y,
            egui::Stroke::new(2.0, selection_color),
        );
    }

    DndReorderResult { swap }
}

/// Displays a modal dialog with a title and message, blocked until dismissed.
#[derive(Default)]
pub struct MessageModal {
    pending: Option<ModalContent>,
}

enum ModalContent {
    /// Simple title + message.
    Message { title: String, message: String },
    /// Rich error display with short message and expandable details.
    Error(anyhow::Error),
}

impl MessageModal {
    pub fn show(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.pending = Some(ModalContent::Message {
            title: title.into(),
            message: message.into(),
        });
    }

    /// Show an error with a short message and expandable detail chain.
    pub fn show_error(&mut self, error: anyhow::Error) {
        self.pending = Some(ModalContent::Error(error));
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        let Some(content) = &self.pending else {
            return;
        };
        let response = egui::Modal::new(egui::Id::new("message_modal")).show(ctx, |ui| {
            ui.set_width(300.0);
            match content {
                ModalContent::Message { title, message } => {
                    ui.heading(title.as_str());
                    ui.label(message.as_str());
                }
                ModalContent::Error(error) => {
                    ui.heading("Error");
                    ui.add_space(4.0);
                    // Full error chain as the default display.
                    ui.label(format!("{error:#}"));

                    // Debug representation (includes backtrace if captured).
                    let debug = format!("{error:?}");
                    let chain = format!("{error:#}");
                    if debug != chain {
                        ui.add_space(4.0);
                        ui.collapsing("Debug", |ui| {
                            ui.monospace(&debug);
                        });
                    }
                }
            }
            ui.add_space(8.0);
            if ui.button("OK").clicked() {
                ui.close();
            }
        });
        if response.should_close() {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    fn modal_harness() -> Harness<'static, MessageModal> {
        Harness::new_ui_state(
            |ui, modal: &mut MessageModal| {
                modal.ui(ui.ctx());
            },
            MessageModal::default(),
        )
    }

    #[test]
    fn show_error_displays_full_chain() {
        // Build a chained error: inner context wraps the root cause.
        let error = anyhow::anyhow!("connection refused")
            .context("failed to send command")
            .context("projector control error");

        let mut harness = modal_harness();
        harness.state_mut().show_error(error);
        harness.step();

        // The full chain should be visible as the default message.
        assert!(harness
            .query_by_label_contains("projector control error")
            .is_some());
        assert!(harness
            .query_by_label_contains("connection refused")
            .is_some());
        assert!(harness.query_by_label("OK").is_some());
    }

    #[test]
    fn show_error_with_backtrace_has_debug_section() {
        // Build an error with context so the debug repr ({:?}) differs from
        // the chain repr ({:#}). The debug repr includes "Caused by:" and
        // backtrace info regardless of RUST_BACKTRACE.
        let error = anyhow::anyhow!("root cause").context("wrapper");

        let debug = format!("{error:?}");
        let chain = format!("{error:#}");

        // Sanity: debug and chain representations should differ (debug has
        // "Caused by:" formatting that chain doesn't).
        assert_ne!(
            debug, chain,
            "debug and chain should differ for chained errors"
        );

        let mut harness = modal_harness();
        harness.state_mut().show_error(error);
        harness.step();

        assert!(harness.query_by_label("Debug").is_some());
    }

    #[test]
    fn show_message_displays_title_and_body() {
        let mut harness = modal_harness();
        harness.state_mut().show("Success", "Config deployed.");
        harness.step();

        assert!(harness.query_by_label("Success").is_some());
        assert!(harness.query_by_label("Config deployed.").is_some());
        assert!(harness.query_by_label("OK").is_some());
    }

    #[test]
    fn snapshot_error_modal() {
        let error = anyhow::anyhow!("connection refused")
            .context("failed to send command")
            .context("projector control error");

        let mut harness = modal_harness();
        harness.state_mut().show_error(error);
        harness.step();
        harness.snapshot("message_modal_error");
    }

    #[test]
    fn snapshot_message_modal() {
        let mut harness = modal_harness();
        harness
            .state_mut()
            .show("Success", "Monitor launched successfully.");
        harness.step();
        harness.snapshot("message_modal_success");
    }
}

/// Handles window close confirmation for egui apps.
///
/// Intercepts the viewport close request, shows a confirmation dialog,
/// and only allows closing when the user confirms.
#[derive(Default)]
pub struct CloseHandler {
    show_confirmation_dialog: bool,
    allowed_to_close: bool,
}

impl CloseHandler {
    pub fn update(&mut self, quit_prompt: &str, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allowed_to_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_confirmation_dialog = true;
        }

        if self.show_confirmation_dialog {
            egui::Window::new(quit_prompt)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("No").clicked() {
                            self.show_confirmation_dialog = false;
                            self.allowed_to_close = false;
                        }

                        if ui.button("Yes").clicked() {
                            self.show_confirmation_dialog = false;
                            self.allowed_to_close = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
        }
    }
}
