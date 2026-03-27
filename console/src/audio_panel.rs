use eframe::egui;

use crate::ui_util::GuiContext;
use gui_common::STATUS_COLORS;
use tunnels::control::MetaCommand;

pub struct AudioPanelState {
    selected_audio: Option<usize>,
    audio_devices: Vec<String>,
}

impl AudioPanelState {
    pub fn new() -> Self {
        let audio_devices = tunnels::audio::AudioInput::devices().unwrap_or_default();
        Self {
            selected_audio: None,
            audio_devices,
        }
    }

    /// Sync the combo box selection from the authoritative show state.
    pub fn sync_from_device_name(&mut self, device_name: &str) {
        self.selected_audio = self.audio_devices.iter().position(|d| d == device_name);
    }

    fn current_audio_device(&self) -> Option<String> {
        self.selected_audio
            .and_then(|i| self.audio_devices.get(i).cloned())
    }
}

pub(crate) struct AudioPanel<'a> {
    pub ctx: GuiContext<'a>,
    pub state: &'a mut AudioPanelState,
    pub current_device: &'a str,
}

impl AudioPanel<'_> {
    pub fn ui(mut self, ui: &mut egui::Ui) {
        ui.heading("Audio Input");
        ui.separator();

        // Status indicator.
        let status_label = format!("Active: {}", self.current_device);
        let status_color = if self.current_device == "Offline" {
            STATUS_COLORS.inactive
        } else {
            STATUS_COLORS.active
        };
        ui.colored_label(status_color, &status_label);
        ui.add_space(8.0);

        let prev_audio = self.state.selected_audio;

        ui.horizontal(|ui| {
            ui.label("Audio Input Device:");
            if ui
                .button("\u{1f504}")
                .on_hover_text("Refresh device list")
                .clicked()
            {
                self.refresh_audio_devices();
            }
        });

        let selected_text = self
            .state
            .selected_audio
            .and_then(|i| self.state.audio_devices.get(i))
            .map_or("Offline", |s| s.as_str());

        egui::ComboBox::from_id_salt("audio_device")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.state.selected_audio, None, "Offline");
                for (i, device) in self.state.audio_devices.iter().enumerate() {
                    ui.selectable_value(&mut self.state.selected_audio, Some(i), device);
                }
            });

        if self.state.selected_audio != prev_audio {
            let device_name = self.state.current_audio_device();
            let _ = self
                .ctx
                .send_command(MetaCommand::SetAudioDevice(device_name));
        }
    }

    fn refresh_audio_devices(&mut self) {
        let prev_device = self.state.current_audio_device();
        match tunnels::audio::AudioInput::devices() {
            Ok(d) => self.state.audio_devices = d,
            Err(e) => {
                self.ctx
                    .report_error(format_args!("Failed to refresh audio devices: {e}"));
                return;
            }
        }
        self.state.selected_audio =
            prev_device.and_then(|name| self.state.audio_devices.iter().position(|d| d == &name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use gui_common::MessageModal;
    use tunnels::control::mock::auto_respond_client;

    /// Build an AudioPanelState with a preset device list, bypassing hardware enumeration.
    fn test_audio_state(devices: Vec<String>, selected: Option<usize>) -> AudioPanelState {
        AudioPanelState {
            selected_audio: selected,
            audio_devices: devices,
        }
    }

    #[test]
    fn render_offline() {
        let client = auto_respond_client();
        let mut modal = MessageModal::default();
        let mut state = test_audio_state(vec![], None);
        let mut harness = Harness::new_ui(|ui| {
            AudioPanel {
                ctx: GuiContext {
                    modal: &mut modal,
                    client: &client,
                },
                state: &mut state,
                current_device: "Offline",
            }
            .ui(ui);
        });
        harness.run();
        harness.snapshot("audio_panel_offline");
    }

    #[test]
    fn render_with_devices() {
        let client = auto_respond_client();
        let mut modal = MessageModal::default();
        let devices = vec![
            "Built-in Microphone".to_string(),
            "Scarlett 2i2 USB".to_string(),
        ];
        let mut state = test_audio_state(devices, Some(1));
        let mut harness = Harness::new_ui(|ui| {
            AudioPanel {
                ctx: GuiContext {
                    modal: &mut modal,
                    client: &client,
                },
                state: &mut state,
                current_device: "Scarlett 2i2 USB",
            }
            .ui(ui);
        });
        harness.run();
        harness.snapshot("audio_panel_with_devices");
    }
}
