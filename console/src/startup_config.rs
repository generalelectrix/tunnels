use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui;
use tunnels::clock_bank::{CLOCKS_PER_WING, MAX_CLOCKS};

/// The maximum number of clock wings the session may configure.
pub const MAX_WINGS: usize = MAX_CLOCKS / CLOCKS_PER_WING;

/// The session configuration chosen at startup.
#[derive(Debug, Clone, Copy)]
pub struct StartupConfig {
    /// Number of clock-control wings, each contributing `CLOCKS_PER_WING` clocks.
    pub n_clock_wings: usize,
}

/// Clamp a requested wing count into the valid `1..=MAX_WINGS` range.
fn clamp_wings(n: usize) -> usize {
    n.clamp(1, MAX_WINGS)
}

struct StartupConfigApp {
    /// Written when the user commits a choice, read after the window closes.
    result: Arc<Mutex<Option<StartupConfig>>>,
    n_clock_wings: usize,
}

impl eframe::App for StartupConfigApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                ui.heading("Tunnels");
                ui.add_space(24.0);

                ui.horizontal(|ui| {
                    ui.label("Clock wings:");
                    ui.add(egui::DragValue::new(&mut self.n_clock_wings).range(1..=MAX_WINGS));
                });
                self.n_clock_wings = clamp_wings(self.n_clock_wings);

                let n_clocks = self.n_clock_wings * CLOCKS_PER_WING;
                ui.add_space(8.0);
                ui.label(format!("{n_clocks} clocks"));

                ui.add_space(24.0);
                if ui.button("Start").clicked() {
                    *self.result.lock().expect("startup config mutex poisoned") =
                        Some(StartupConfig {
                            n_clock_wings: self.n_clock_wings,
                        });
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

/// Run the startup config splash, blocking until the user starts a session or
/// closes the window. Returns the chosen config, or `None` if the window was
/// closed without starting.
pub fn run_startup_config() -> Result<Option<StartupConfig>> {
    let result: Arc<Mutex<Option<StartupConfig>>> = Arc::new(Mutex::new(None));
    let app_result = result.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([320.0, 220.0])
            .with_resizable(false)
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
        ..Default::default()
    };

    eframe::run_native(
        "Tunnels",
        options,
        Box::new(move |cc| {
            stage_theme::apply(&cc.egui_ctx);
            Ok(Box::new(StartupConfigApp {
                result: app_result,
                n_clock_wings: 1,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe startup config window failed: {e}"))?;

    let inner = Arc::try_unwrap(result)
        .map_err(|_| anyhow::anyhow!("startup config Arc still shared"))?
        .into_inner()
        .expect("startup config mutex poisoned");
    Ok(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_wings_to_valid_range() {
        assert_eq!(clamp_wings(0), 1);
        assert_eq!(clamp_wings(1), 1);
        assert_eq!(clamp_wings(MAX_WINGS), MAX_WINGS);
        assert_eq!(clamp_wings(MAX_WINGS + 5), MAX_WINGS);
    }
}
