use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;
use tunnels::animation_visualizer::VisualizerPanelState;
use tunnels::gui_state::SharedGuiState;

/// Render the animation visualizer, either embedded in the tab or showing
/// a "detached" placeholder with a reattach button.
pub(crate) fn ui(
    ui: &mut egui::Ui,
    gui_state: &SharedGuiState,
    panel: &Arc<Mutex<VisualizerPanelState>>,
    detached: &Arc<AtomicBool>,
) {
    if detached.load(Ordering::Relaxed) {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label("Visualizer is in a separate window.");
            if ui.button("Reattach").clicked() {
                detached.store(false, Ordering::Relaxed);
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui.button("Detach").clicked() {
                        detached.store(true, Ordering::Relaxed);
                    }
                },
            );
        });
        let snapshot = gui_state.animation_state.load();
        if let Ok(mut panel) = panel.lock() {
            panel.ui(ui, &snapshot);
        }
    }
}

/// Show the detached animation visualizer as a separate OS window via
/// `show_viewport_deferred`.
pub(crate) fn show_detached_viewport(
    ctx: &egui::Context,
    gui_state: &SharedGuiState,
    panel: &Arc<Mutex<VisualizerPanelState>>,
    detached: &Arc<AtomicBool>,
) {
    let gui_state = gui_state.clone();
    let detached_flag = detached.clone();
    let panel = panel.clone();
    ctx.show_viewport_deferred(
        egui::ViewportId::from_hash_of("animation_visualizer"),
        egui::ViewportBuilder::default()
            .with_title("Animation Visualizer")
            .with_inner_size(egui::vec2(600.0, 400.0)),
        move |ctx, _class| {
            let Ok(mut panel) = panel.lock() else { return };
            let snapshot = gui_state.animation_state.load();
            egui::CentralPanel::default().show(ctx, |ui| {
                panel.ui(ui, &snapshot);
            });
            if ctx.input(|i| i.viewport().close_requested()) {
                detached_flag.store(false, Ordering::Relaxed);
            }
        },
    );
}
