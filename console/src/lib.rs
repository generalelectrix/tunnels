pub mod admin_panel;
mod animation_panel;
mod audio_panel;
pub mod bootstrap_controller;
mod midi_panel;
mod ui_util;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use eframe::egui;

use admin_panel::{AdminPanelState, AdminService};
use audio_panel::AudioPanelState;
use gui_common::envelope_viewer::EnvelopeViewerState;
use gui_common::tracked::TrackedBool;
use gui_common::{CloseHandler, MessageModal, clock_panel};
use midi_panel::{MidiPanel, MidiPanelState};
use tunnels::animation_visualizer::VisualizerPanelState;
use tunnels::audio::EnvelopeStreams;
use tunnels::control::{CommandClient, MetaCommand};
use tunnels::gui_state::SharedGuiState;
use tunnels_lib::repaint::RepaintSignal;
use ui_util::GuiContext;

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    Midi,
    Audio,
    Animation,
    Clients,
}

pub struct ConfigApp {
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
    envelope_viewer: EnvelopeViewerState,
    envelope_streams_rx: Receiver<EnvelopeStreams>,
    close_handler: CloseHandler,
    modal: MessageModal,
    active_tab: Tab,
    visualizer_active: TrackedBool,
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
        self.visualizer_active
            .update(detached || self.active_tab == Tab::Animation)
            .if_changed(|v| {
                let _ = self
                    .client
                    .send_command(MetaCommand::SetVisualizerActive(v));
            });

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
                    audio_panel::render_audio_panel(
                        ui,
                        GuiContext {
                            modal: &mut self.modal,
                            client: &self.client,
                        },
                        &mut self.audio_panel,
                        &audio_state,
                    );

                    if audio_state.device_name != tunnels::audio::OFFLINE_DEVICE_NAME {
                        // Drain new envelope streams from the audio reconnect
                        // thread. If multiple have accumulated, the most recent
                        // wins — `set_envelope_streams` fully resets the viewer.
                        while let Ok(envelope_streams) = self.envelope_streams_rx.try_recv() {
                            self.envelope_viewer.set_envelope_streams(envelope_streams);
                        }

                        ui.add_space(8.0);
                        ui.separator();

                        self.envelope_viewer.ui(ui);
                    } else {
                        self.envelope_viewer.set_open(false);
                    }
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

impl ConfigApp {
    /// Build the console GUI. Must be called from inside the eframe creator
    /// closure so the `RepaintSignal` can wrap `cc.egui_ctx`.
    pub fn new(
        client: CommandClient,
        gui_state: SharedGuiState,
        admin_service: Arc<dyn AdminService>,
        hostname: String,
        repaint: RepaintSignal,
        envelope_streams_rx: Receiver<EnvelopeStreams>,
    ) -> Self {
        let audio_state = gui_state.audio_state.load();
        let devices = tunnels::audio::AudioInput::devices().unwrap_or_default();
        let mut audio_panel = AudioPanelState::new(devices);
        audio_panel.sync_from_device_name(&audio_state.device_name);
        drop(audio_state);

        let admin_panel = AdminPanelState::new(admin_service.clone(), hostname, repaint);

        Self {
            midi_panel: MidiPanelState::default(),
            audio_panel,
            admin_panel,
            admin_service,
            visualizer_panel: Arc::new(Mutex::new(VisualizerPanelState::default())),
            visualizer_detached: Arc::new(AtomicBool::new(false)),
            envelope_viewer: EnvelopeViewerState::new(),
            envelope_streams_rx,
            close_handler: CloseHandler::default(),
            modal: MessageModal::default(),
            client,
            active_tab: Tab::default(),
            visualizer_active: TrackedBool::new(false),
            gui_state,
        }
    }
}

/// Default native options for the console window.
pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([864.0, 600.0])
            .with_icon(std::sync::Arc::new(egui::IconData::default())),
        ..Default::default()
    }
}
