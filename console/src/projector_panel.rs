//! egui-based panel for controlling projectors attached to render clients.

use std::sync::Arc;

use anyhow::Result;
use eframe::egui;
use tunnels_lib::projector::{PowerState, ProjectorStatus};

use crate::projector_controller::ProjectorController;
use crate::ui_util::GuiContext;

/// Abstraction over projector operations so we can mock in tests.
pub trait ProjectorService: Send + Sync {
    fn projectors(&self) -> Vec<String>;
    fn get_status(&self, name: &str) -> Result<ProjectorStatus>;
    fn connect(&self, name: &str, port_path: &str) -> Result<()>;
    fn disconnect(&self, name: &str) -> Result<()>;
    fn set_power(&self, name: &str, on: bool) -> Result<()>;
    fn set_av_mute(&self, name: &str, on: bool) -> Result<()>;
    fn set_eco_mode(&self, name: &str, eco: bool) -> Result<()>;
}

impl ProjectorService for ProjectorController {
    fn projectors(&self) -> Vec<String> {
        self.list()
    }
    fn get_status(&self, name: &str) -> Result<ProjectorStatus> {
        self.get_status(name)
    }
    fn connect(&self, name: &str, port_path: &str) -> Result<()> {
        self.connect(name, port_path)
    }
    fn disconnect(&self, name: &str) -> Result<()> {
        self.disconnect(name)
    }
    fn set_power(&self, name: &str, on: bool) -> Result<()> {
        self.set_power(name, on)
    }
    fn set_av_mute(&self, name: &str, on: bool) -> Result<()> {
        self.set_av_mute(name, on)
    }
    fn set_eco_mode(&self, name: &str, eco: bool) -> Result<()> {
        self.set_eco_mode(name, eco)
    }
}

pub struct ProjectorPanelState {
    projector_service: Arc<dyn ProjectorService>,
    selected_projector: Option<String>,
    cached_status: Option<ProjectorStatus>,
}

impl ProjectorPanelState {
    pub fn new(projector_service: Arc<dyn ProjectorService>) -> Self {
        Self {
            projector_service,
            selected_projector: None,
            cached_status: None,
        }
    }

    fn select_projector(&mut self, name: String) {
        if self.selected_projector.as_ref() == Some(&name) {
            return;
        }
        self.selected_projector = Some(name);
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        let Some(ref name) = self.selected_projector else {
            return;
        };
        match self.projector_service.get_status(name) {
            Ok(status) => self.cached_status = Some(status),
            Err(e) => log::warn!("Failed to refresh projector status: {e}"),
        }
    }

    /// Dispatch a projector command. On success, refreshes cached status.
    fn dispatch<App>(
        &self,
        gui: &mut GuiContext<'_, App>,
        label: &str,
        action: impl FnOnce(&dyn ProjectorService, &str) -> Result<()> + Send + 'static,
    ) where
        App: gui_common::background_task::Project<Self> + 'static,
    {
        let Some(ref name) = self.selected_projector else {
            return;
        };
        let name = name.clone();
        let service = self.projector_service.clone();
        gui.dispatch::<Self, _>(
            label,
            move || action(&*service, &name),
            |panel, ()| panel.refresh_status(),
        );
    }

    pub(crate) fn render<App>(
        &mut self,
        ctx: &egui::Context,
        projectors: &[String],
        gui: &mut GuiContext<'_, App>,
    ) where
        App: gui_common::background_task::Project<Self> + 'static,
    {
        // Invalidate selection if the selected projector has disappeared.
        if let Some(ref name) = self.selected_projector
            && !projectors.iter().any(|p| p == name)
        {
            self.selected_projector = None;
            self.cached_status = None;
        }

        let panel_frame = egui::Frame::central_panel(&ctx.style());

        let side_panel_width = 180.0;
        egui::SidePanel::left("projectors_panel")
            .min_width(side_panel_width)
            .max_width(side_panel_width)
            .frame(panel_frame)
            .show(ctx, |ui| {
                ui.heading("Projectors");
                ui.separator();

                if projectors.is_empty() {
                    ui.label("No projectors found.");
                }

                for name in projectors {
                    let selected = self.selected_projector.as_ref() == Some(name);
                    if ui.selectable_label(selected, name).clicked() {
                        self.select_projector(name.clone());
                    }
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(name) = self.selected_projector.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a projector to control.");
                });
                return;
            };

            ui.heading(format!("Projector: {name}"));
            ui.separator();

            let Some(ref status) = self.cached_status else {
                ui.label("No status available.");
                if ui.button("Refresh").clicked() {
                    self.refresh_status();
                }
                return;
            };
            let status = status.clone();

            self.render_connection(ui, gui, &status);

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            if status.connected_port.is_some() {
                render_projector_state(ui, &status);

                ui.add_space(16.0);
                ui.separator();

                let is_on = status.power == PowerState::On;

                ui.heading("Controls");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("Power On").clicked() {
                        self.dispatch(gui, "Power On", |svc, name| svc.set_power(name, true));
                    }
                    if ui.button("Power Off").clicked() {
                        self.dispatch(gui, "Power Off", |svc, name| svc.set_power(name, false));
                    }
                });

                ui.add_space(4.0);

                ui.add_enabled_ui(is_on, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("AV Mute On").clicked() {
                            self.dispatch(gui, "AV Mute On", |svc, name| {
                                svc.set_av_mute(name, true)
                            });
                        }
                        if ui.button("AV Mute Off").clicked() {
                            self.dispatch(gui, "AV Mute Off", |svc, name| {
                                svc.set_av_mute(name, false)
                            });
                        }
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        if ui.button("ECO").clicked() {
                            self.dispatch(gui, "ECO Mode", |svc, name| {
                                svc.set_eco_mode(name, true)
                            });
                        }
                        if ui.button("Standard").clicked() {
                            self.dispatch(gui, "Standard Mode", |svc, name| {
                                svc.set_eco_mode(name, false)
                            });
                        }
                    });
                });
            }

            ui.add_space(16.0);

            if ui.button("Refresh Status").clicked() {
                self.refresh_status();
            }
        });
    }

    fn render_connection<App>(
        &self,
        ui: &mut egui::Ui,
        gui: &mut GuiContext<'_, App>,
        status: &ProjectorStatus,
    ) where
        App: gui_common::background_task::Project<Self> + 'static,
    {
        ui.heading("Serial Port");
        ui.add_space(4.0);

        if let Some(ref port) = status.connected_port {
            ui.horizontal(|ui| {
                ui.label(format!("Connected: {port}"));
                if ui.button("Disconnect").clicked() {
                    self.dispatch(gui, "Disconnect", |svc, name| svc.disconnect(name));
                }
            });
        } else if status.available_ports.is_empty() {
            ui.label("No serial ports detected.");
        } else {
            ui.label("Available ports:");
            let ports: Vec<String> = status.available_ports.clone();
            for port in &ports {
                ui.horizontal(|ui| {
                    ui.label(port);
                    if ui.button("Connect").clicked() {
                        let port = port.clone();
                        self.dispatch(gui, "Connect", move |svc, name| svc.connect(name, &port));
                    }
                });
            }
        }
    }
}

fn render_projector_state(ui: &mut egui::Ui, status: &ProjectorStatus) {
    egui::Grid::new("projector_status_grid")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.label("Power:");
            ui.label(power_state_label(&status.power));
            ui.end_row();

            ui.label("A/V Mute:");
            ui.label(if status.av_mute { "On" } else { "Off" });
            ui.end_row();

            ui.label("Eco Mode:");
            ui.label(if status.eco_mode { "ECO" } else { "Standard" });
            ui.end_row();

            if let Some(ref mode) = status.color_mode {
                ui.label("Color Mode:");
                ui.label(mode.as_str());
                ui.end_row();
            }

            ui.label("Lamp Hours:");
            ui.label(format!("{}", status.lamp_hours));
            ui.end_row();

            ui.label("Error:");
            ui.label(if status.error_code == 0 {
                "None".to_string()
            } else {
                format!("0x{:02X}", status.error_code)
            });
            ui.end_row();
        });
}

fn power_state_label(state: &PowerState) -> &'static str {
    match state {
        PowerState::Off => "Off",
        PowerState::On => "On",
        PowerState::WarmingUp => "Warming Up",
        PowerState::CoolingDown => "Cooling Down",
        PowerState::Standby => "Standby",
        PowerState::Error => "Error",
    }
}
