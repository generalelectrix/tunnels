pub mod admin_panel;
mod animation_panel;
mod audio_panel;
pub mod bootstrap_controller;
mod envelope_viewer;
mod midi_panel;
mod ui_util;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui;

use admin_panel::{AdminPanelState, AdminService};
use audio_panel::AudioPanelState;
use gui_common::{CloseHandler, MessageModal, audio_panel::AudioSnapshot, clock_panel};
use midi_panel::{MidiPanel, MidiPanelState};
use tunnels::animation_visualizer::VisualizerPanelState;
use tunnels::control::{CommandClient, MetaCommand};
use tunnels::gui_state::SharedGuiState;
use ui_util::GuiContext;

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    Midi,
    Audio,
    Animation,
    Clients,
}

struct ConfigApp {
    client: CommandClient,
    midi_panel: MidiPanelState,
    audio_panel: AudioPanelState,
    admin_panel: AdminPanelState,
    admin_service: Arc<dyn AdminService>,
    /// Behind Arc<Mutex<>> because this state is shared between the embedded
    /// Animation tab and the detached viewport (which runs on a separate
    /// thread via show_viewport_deferred). Only one renders at a time, so the
    /// mutex is never contended.
    visualizer_panel: Arc<Mutex<VisualizerPanelState>>,
    /// Shared with the deferred viewport closure so it can signal "close" back
    /// to the main thread. Arc<AtomicBool> because the deferred closure is
    /// 'static + Send + Sync and can't hold a reference to ConfigApp fields.
    visualizer_detached: Arc<AtomicBool>,
    envelope_viewer: envelope_viewer::EnvelopeViewerState,
    close_handler: CloseHandler,
    modal: MessageModal,
    active_tab: Tab,
    gui_state: SharedGuiState,
}

impl eframe::App for ConfigApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.close_handler.update("Quit Tunnels?", ctx);

        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Midi, "MIDI");
                ui.selectable_value(&mut self.active_tab, Tab::Audio, "Audio");
                ui.selectable_value(&mut self.active_tab, Tab::Animation, "Animation");
                ui.selectable_value(&mut self.active_tab, Tab::Clients, "Clients");
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
            // Admin panel draws its own SidePanel + CentralPanel.
            let clients = self.admin_service.clients();
            self.admin_panel.render(ctx, &clients);
        } else {
            egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
                Tab::Midi => {
                    let midi_slots = self.gui_state.midi_slots.load();
                    let mut ctx = GuiContext {
                        modal: &mut self.modal,
                        client: &self.client,
                    };
                    MidiPanel {
                        commands: &mut ctx,
                        state: &mut self.midi_panel,
                        slots: &midi_slots,
                    }
                    .ui(ui);

                    ui.add_space(16.0);
                    ui.separator();
                    let clock_running =
                        self.gui_state.clock_service_running.load(Ordering::Relaxed);
                    if let Some(action) = clock_panel::clock_service_ui(ui, clock_running) {
                        let cmd = match action {
                            clock_panel::ClockServiceAction::Start => {
                                MetaCommand::StartClockService
                            }
                            clock_panel::ClockServiceAction::Stop => MetaCommand::StopClockService,
                        };
                        let mut ctx = GuiContext {
                            modal: &mut self.modal,
                            client: &self.client,
                        };
                        let _ = ctx.send_command(cmd);
                    }
                }
                Tab::Audio => {
                    let audio_state = self.gui_state.audio_state.load();
                    let audio_device = self.gui_state.audio_device.load();
                    let snapshot = AudioSnapshot {
                        device_name: audio_device.as_ref().clone(),
                        filter_cutoff_hz: audio_state.filter_cutoff_hz,
                        envelope_attack: audio_state.envelope_attack,
                        envelope_release: audio_state.envelope_release,
                        output_smoothing: audio_state.output_smoothing,
                        gain_linear: audio_state.gain_linear,
                        auto_trim_enabled: audio_state.auto_trim_enabled,
                        active_band: audio_state.active_band,
                        norm_floor_halflife: audio_state.norm_floor_halflife,
                        norm_ceiling_halflife: audio_state.norm_ceiling_halflife,
                        norm_floor_mode: audio_state.norm_floor_mode,
                        norm_ceiling_mode: audio_state.norm_ceiling_mode,
                    };
                    audio_panel::render_audio_panel(
                        ui,
                        GuiContext {
                            modal: &mut self.modal,
                            client: &self.client,
                        },
                        &mut self.audio_panel,
                        &snapshot,
                    );

                    ui.add_space(8.0);
                    ui.separator();

                    // Envelope viewer: read the shared handle from gui_state.
                    let envelope_history_guard = self.gui_state.envelope_history.load();
                    let envelope_history = envelope_history_guard.as_ref().as_ref();
                    self.envelope_viewer
                        .ui(ui, envelope_history, audio_state.update_rate);
                }
                Tab::Animation => {
                    animation_panel::ui(
                        ui,
                        &self.gui_state,
                        &self.visualizer_panel,
                        &self.visualizer_detached,
                    );
                }
                Tab::Clients => {} // handled above
            });
        }

        self.modal.ui(ctx);
    }
}

pub fn run_config_gui(
    client: CommandClient,
    gui_state: SharedGuiState,
    admin_service: Arc<dyn AdminService>,
    hostname: String,
) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([864.0, 600.0])
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
        ..Default::default()
    };
    let audio_device = gui_state.audio_device.load();
    let devices = tunnels::audio::AudioInput::devices().unwrap_or_default();
    let mut audio_panel = AudioPanelState::new(devices);
    audio_panel.sync_from_device_name(&audio_device);

    let admin_panel = AdminPanelState::new(admin_service.clone(), hostname);

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
                visualizer_panel: Arc::new(Mutex::new(VisualizerPanelState::default())),
                visualizer_detached: Arc::new(AtomicBool::new(false)),
                envelope_viewer: envelope_viewer::EnvelopeViewerState::new(),
                close_handler: CloseHandler::default(),
                modal: MessageModal::default(),
                client,
                active_tab: Tab::default(),
                gui_state,
            }))
        }),
    )
    .unwrap();
    Ok(())
}
