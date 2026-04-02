pub mod admin_panel;
mod animation_panel;
mod audio_panel;
pub mod bootstrap_controller;
mod midi_panel;
pub mod projector_controller;
pub mod projector_panel;
mod ui_util;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui;
use gui_common::background_task::{BlockingBackgroundTask, OptionTaskExt};
use gui_common::impl_project;

use admin_panel::{AdminPanelState, AdminService};
use audio_panel::{AudioPanel, AudioPanelState};
use gui_common::{CloseHandler, MessageModal};
use midi_panel::{MidiPanel, MidiPanelState};
use projector_panel::{ProjectorPanelState, ProjectorService};
use tunnels::animation_visualizer::VisualizerPanelState;
use tunnels::control::CommandClient;
use tunnels::gui_state::SharedGuiState;
use ui_util::GuiContext;

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    Midi,
    Audio,
    Animation,
    Clients,
    Projectors,
}

struct ConfigApp {
    client: CommandClient,
    midi_panel: MidiPanelState,
    audio_panel: AudioPanelState,
    admin_panel: AdminPanelState,
    admin_service: Arc<dyn AdminService>,
    projector_panel: ProjectorPanelState,
    projector_service: Arc<dyn ProjectorService>,
    visualizer_panel: Arc<Mutex<VisualizerPanelState>>,
    visualizer_detached: Arc<AtomicBool>,
    close_handler: CloseHandler,
    modal: MessageModal,
    active_tab: Tab,
    gui_state: SharedGuiState,
    task: Option<BlockingBackgroundTask<Self>>,
}

// Project impls — allow panels to dispatch background tasks targeting their own state.
impl_project!(ConfigApp, projector_panel: ProjectorPanelState);
impl_project!(ConfigApp, admin_panel: AdminPanelState);

impl gui_common::UserNotify for ConfigApp {
    fn notify(&mut self, title: &str, message: &str) {
        self.modal.show(title, message);
    }
    fn notify_error(&mut self, error: anyhow::Error) {
        self.modal.show_error(error);
    }
}

impl eframe::App for ConfigApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll and apply background task results before splitting borrows.
        self.task.poll(ctx).if_complete(self);

        self.close_handler.update("Quit Tunnels?", ctx);

        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Midi, "MIDI");
                ui.selectable_value(&mut self.active_tab, Tab::Audio, "Audio");
                ui.selectable_value(&mut self.active_tab, Tab::Animation, "Animation");
                ui.selectable_value(&mut self.active_tab, Tab::Clients, "Clients");
                ui.selectable_value(&mut self.active_tab, Tab::Projectors, "Projectors");
            });
        });

        // Notify the show when the visualizer is visible (either tab or detached window).
        let detached = self.visualizer_detached.load(Ordering::Relaxed);
        self.gui_state.visualizer_active.store(
            detached || self.active_tab == Tab::Animation,
            Ordering::Relaxed,
        );

        // Detached animation visualizer -- separate OS window via deferred viewport.
        if detached {
            animation_panel::show_detached_viewport(
                ctx,
                &self.gui_state,
                &self.visualizer_panel,
                &self.visualizer_detached,
            );
        }

        if self.active_tab == Tab::Clients {
            let clients = self.admin_service.clients();
            let mut gui = GuiContext {
                modal: &mut self.modal,
                client: &self.client,
                task: &mut self.task,
            };
            self.admin_panel.render(ctx, &clients, &mut gui);
        } else if self.active_tab == Tab::Projectors {
            let projectors = self.projector_service.projectors();
            let mut gui = GuiContext {
                modal: &mut self.modal,
                client: &self.client,
                task: &mut self.task,
            };
            self.projector_panel.render(ctx, &projectors, &mut gui);
        } else {
            egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
                Tab::Midi => {
                    let midi_slots = self.gui_state.midi_slots.load();
                    let mut gui = GuiContext {
                        modal: &mut self.modal,
                        client: &self.client,
                        task: &mut self.task,
                    };
                    MidiPanel {
                        commands: &mut gui,
                        state: &mut self.midi_panel,
                        slots: &midi_slots,
                    }
                    .ui(ui);
                }
                Tab::Audio => {
                    let audio_device = self.gui_state.audio_device.load();
                    let clock_service_running =
                        self.gui_state.clock_service_running.load(Ordering::Relaxed);
                    AudioPanel {
                        ctx: GuiContext {
                            modal: &mut self.modal,
                            client: &self.client,
                            task: &mut self.task,
                        },
                        state: &mut self.audio_panel,
                        current_device: &audio_device,
                        clock_service_running,
                    }
                    .ui(ui);
                }
                Tab::Animation => {
                    animation_panel::ui(
                        ui,
                        &self.gui_state,
                        &self.visualizer_panel,
                        &self.visualizer_detached,
                    );
                }
                Tab::Clients | Tab::Projectors => {} // handled above
            });
        }

        self.modal.ui(ctx);
    }
}

pub fn run_config_gui(
    client: CommandClient,
    gui_state: SharedGuiState,
    admin_service: Arc<dyn AdminService>,
    projector_service: Arc<dyn ProjectorService>,
    hostname: String,
) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
        ..Default::default()
    };
    let audio_device = gui_state.audio_device.load();
    let mut audio_panel = AudioPanelState::new();
    audio_panel.sync_from_device_name(&audio_device);

    let admin_panel = AdminPanelState::new(admin_service.clone(), hostname);
    let projector_panel = ProjectorPanelState::new(projector_service.clone());

    eframe::run_native(
        "Tunnels",
        options,
        Box::new(move |cc| {
            stage_theme::apply(&cc.egui_ctx);
            Ok(Box::new(ConfigApp {
                midi_panel: MidiPanelState::default(),
                audio_panel,
                admin_panel,
                admin_service,
                projector_panel,
                projector_service,
                visualizer_panel: Arc::new(Mutex::new(VisualizerPanelState::default())),
                visualizer_detached: Arc::new(AtomicBool::new(false)),
                close_handler: CloseHandler::default(),
                modal: MessageModal::default(),
                client,
                active_tab: Tab::default(),
                gui_state,
                task: None,
            }))
        }),
    )
    .unwrap();
    Ok(())
}
