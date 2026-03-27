mod animation_panel;
mod audio_panel;
mod midi_panel;
mod ui_util;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui;

use audio_panel::{AudioPanel, AudioPanelState};
use gui_common::{CloseHandler, MessageModal};
use midi_panel::{MidiPanel, MidiPanelState};
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
}

struct ConfigApp {
    client: CommandClient,
    midi_panel: MidiPanelState,
    audio_panel: AudioPanelState,
    /// Behind Arc<Mutex<>> because this state is shared between the embedded
    /// Animation tab and the detached viewport (which runs on a separate
    /// thread via show_viewport_deferred). Only one renders at a time, so the
    /// mutex is never contended.
    visualizer_panel: Arc<Mutex<VisualizerPanelState>>,
    /// Shared with the deferred viewport closure so it can signal "close" back
    /// to the main thread. Arc<AtomicBool> because the deferred closure is
    /// 'static + Send + Sync and can't hold a reference to ConfigApp fields.
    visualizer_detached: Arc<AtomicBool>,
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
            }
            Tab::Audio => {
                let audio_device = self.gui_state.audio_device.load();
                AudioPanel {
                    ctx: GuiContext {
                        modal: &mut self.modal,
                        client: &self.client,
                    },
                    state: &mut self.audio_panel,
                    current_device: &audio_device,
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
        });

        self.modal.ui(ctx);
    }
}

pub fn run_config_gui(client: CommandClient, gui_state: SharedGuiState) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([600.0, 500.0]),
        ..Default::default()
    };
    let audio_device = gui_state.audio_device.load();
    let mut audio_panel = AudioPanelState::new();
    audio_panel.sync_from_device_name(&audio_device);

    eframe::run_native(
        "Tunnels",
        options,
        Box::new(move |cc| {
            stage_theme::apply(&cc.egui_ctx);
            Ok(Box::new(ConfigApp {
                midi_panel: MidiPanelState::new(),
                audio_panel,
                visualizer_panel: Arc::new(Mutex::new(VisualizerPanelState::default())),
                visualizer_detached: Arc::new(AtomicBool::new(false)),
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
