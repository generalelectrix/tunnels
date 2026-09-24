use eframe::egui;
use std::time::Duration;
pub use tunnels_audio::AudioSnapshot;
use tunnels_audio::OFFLINE_DEVICE_NAME;
use tunnels_audio::processor::{NUM_OUTPUT_BANDS, output_band_labels};

/// The sample rate the band labels describe until a device opens.
const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// Abstraction over project-specific command dispatch for audio panels.
pub trait AudioCommands {
    fn set_device(&mut self, device: Option<String>);
    fn set_envelope_attack(&mut self, duration: Duration);
    fn set_envelope_release(&mut self, duration: Duration);
    fn set_output_smoothing(&mut self, duration: Duration);
    fn set_active_band(&mut self, band: u32);
    fn set_norm_floor_halflife(&mut self, halflife: Duration);
    fn set_norm_ceiling_halflife(&mut self, halflife: Duration);
    fn reset_parameters(&mut self);
    fn list_devices(&mut self) -> Vec<String>;
}

pub struct AudioPanelState {
    selected_audio: Option<usize>,
    audio_devices: Vec<String>,
    /// Band labels, and the sample rate they describe. Band edges are
    /// octaves of the rate, so they only change when a device does.
    band_labels: [String; NUM_OUTPUT_BANDS],
    labelled_rate: u32,
}

impl AudioPanelState {
    pub fn new(devices: Vec<String>) -> Self {
        Self {
            selected_audio: None,
            audio_devices: devices,
            band_labels: output_band_labels(DEFAULT_SAMPLE_RATE),
            labelled_rate: DEFAULT_SAMPLE_RATE,
        }
    }

    /// Report the sample rate of the device that just opened. Band edges are
    /// octaves of it, so the labels are rebuilt when it changes.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate != self.labelled_rate {
            self.band_labels = output_band_labels(sample_rate);
            self.labelled_rate = sample_rate;
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

pub struct AudioPanel<'a, C: AudioCommands> {
    pub commands: &'a mut C,
    pub state: &'a mut AudioPanelState,
    pub snapshot: &'a AudioSnapshot,
}

impl<C: AudioCommands> AudioPanel<'_, C> {
    pub fn ui(mut self, ui: &mut egui::Ui) {
        self.device_selection(ui);

        if self.snapshot.device_name == OFFLINE_DEVICE_NAME {
            return;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Two-column layout: Input sizes to content, Envelope takes the rest.
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                self.input_controls(ui);
                ui.add_space(4.0);
                if ui.button("Reset All").clicked() {
                    self.commands.reset_parameters();
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());
                self.envelope_controls(ui);
            });
        });
    }

    fn device_selection(&mut self, ui: &mut egui::Ui) {
        let prev_audio = self.state.selected_audio;
        // Use the snapshot's device name for display — it's the authoritative
        // state from the show, already a separate String with no borrow on self.
        let selected_text = &self.snapshot.device_name;

        ui.horizontal(|ui| {
            ui.label("Audio Input Device:");
            egui::ComboBox::from_id_salt("audio_device")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.state.selected_audio, None, OFFLINE_DEVICE_NAME);
                    for (i, device) in self.state.audio_devices.iter().enumerate() {
                        ui.selectable_value(&mut self.state.selected_audio, Some(i), device);
                    }
                });
            if ui
                .button("\u{1f504}")
                .on_hover_text("Refresh device list")
                .clicked()
            {
                self.refresh_audio_devices();
            }
        });

        if self.state.selected_audio != prev_audio {
            let device_name = self.state.current_audio_device();
            self.commands.set_device(device_name);
        }
    }

    fn input_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Input");
        ui.add_space(4.0);

        egui::Grid::new("input_controls_grid").show(ui, |ui| {
            // Band selector.
            ui.label("Active band:");
            let mut band = self.snapshot.active_band;
            let labels = &self.state.band_labels;
            let selected_text = labels.get(band as usize).unwrap_or(&labels[0]);
            egui::ComboBox::from_id_salt("active_band")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (i, label) in labels.iter().enumerate() {
                        ui.selectable_value(&mut band, i as u32, label);
                    }
                });
            if band != self.snapshot.active_band {
                self.commands.set_active_band(band);
            }
            ui.end_row();
        });
    }

    fn envelope_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Envelope");
        ui.add_space(4.0);

        egui::Grid::new("envelope_controls_grid").show(ui, |ui| {
            // Attack.
            ui.label("Attack:");
            let mut attack_ms = self.snapshot.envelope_attack.as_secs_f32() * 1000.0;
            if ui
                .add(
                    egui::Slider::new(&mut attack_ms, 1.0..=256.0)
                        .suffix(" ms")
                        .logarithmic(true),
                )
                .changed()
            {
                self.commands
                    .set_envelope_attack(Duration::from_secs_f32(attack_ms / 1000.0));
            }
            ui.end_row();

            // Release.
            ui.label("Release:");
            let mut release_ms = self.snapshot.envelope_release.as_secs_f32() * 1000.0;
            if ui
                .add(
                    egui::Slider::new(&mut release_ms, 1.0..=1000.0)
                        .suffix(" ms")
                        .logarithmic(true),
                )
                .changed()
            {
                self.commands
                    .set_envelope_release(Duration::from_secs_f32(release_ms / 1000.0));
            }
            ui.end_row();

            // Smoothing.
            ui.label("Smoothing:");
            let mut smooth_ms = self.snapshot.output_smoothing.as_secs_f32() * 1000.0;
            if ui
                .add(egui::Slider::new(&mut smooth_ms, 0.0..=50.0).suffix(" ms"))
                .changed()
            {
                self.commands
                    .set_output_smoothing(Duration::from_secs_f32(smooth_ms / 1000.0));
            }
            ui.end_row();

            // The normalizer's two memories, both half-lives in seconds.
            ui.label("Floor Memory:");
            let mut floor_hl_s = self.snapshot.norm_floor_halflife.as_secs_f32();
            if ui
                .add(
                    egui::Slider::new(&mut floor_hl_s, 0.5..=30.0)
                        .suffix(" s")
                        .logarithmic(true),
                )
                .changed()
            {
                self.commands
                    .set_norm_floor_halflife(Duration::from_secs_f32(floor_hl_s));
            }
            ui.end_row();

            ui.label("Peak Memory:");
            let mut ceil_hl_s = self.snapshot.norm_ceiling_halflife.as_secs_f32();
            if ui
                .add(
                    egui::Slider::new(&mut ceil_hl_s, 0.5..=60.0)
                        .suffix(" s")
                        .logarithmic(true),
                )
                .changed()
            {
                self.commands
                    .set_norm_ceiling_halflife(Duration::from_secs_f32(ceil_hl_s));
            }
            ui.end_row();
        });
    }

    fn refresh_audio_devices(&mut self) {
        let prev_device = self.state.current_audio_device();
        self.state.audio_devices = self.commands.list_devices();
        self.state.selected_audio =
            prev_device.and_then(|name| self.state.audio_devices.iter().position(|d| d == &name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAudioCommands {
        devices: Vec<String>,
    }

    impl MockAudioCommands {
        fn new(devices: Vec<String>) -> Self {
            Self { devices }
        }
    }

    impl AudioCommands for MockAudioCommands {
        fn set_device(&mut self, _device: Option<String>) {}
        fn set_envelope_attack(&mut self, _duration: Duration) {}
        fn set_envelope_release(&mut self, _duration: Duration) {}
        fn set_output_smoothing(&mut self, _duration: Duration) {}
        fn set_active_band(&mut self, _band: u32) {}
        fn set_norm_floor_halflife(&mut self, _halflife: Duration) {}
        fn set_norm_ceiling_halflife(&mut self, _halflife: Duration) {}
        fn reset_parameters(&mut self) {}
        fn list_devices(&mut self) -> Vec<String> {
            self.devices.clone()
        }
    }

    fn default_snapshot() -> AudioSnapshot {
        AudioSnapshot::default()
    }

    #[test]
    fn render_offline() {
        use egui_kittest::Harness;
        let mut commands = MockAudioCommands::new(vec![]);
        let mut state = AudioPanelState::new(vec![]);
        let snapshot = default_snapshot();
        let mut harness = Harness::new_ui(|ui| {
            AudioPanel {
                commands: &mut commands,
                state: &mut state,
                snapshot: &snapshot,
            }
            .ui(ui);
        });
        harness.run();
        harness.snapshot("audio_panel_offline");
    }

    #[test]
    fn render_with_devices() {
        use egui_kittest::Harness;
        let devices = vec![
            "Built-in Microphone".to_string(),
            "Scarlett 2i2 USB".to_string(),
        ];
        let mut commands = MockAudioCommands::new(devices.clone());
        let mut state = AudioPanelState::new(devices);
        state.selected_audio = Some(1);
        let snapshot = AudioSnapshot {
            device_name: "Scarlett 2i2 USB".to_string(),
            ..default_snapshot()
        };
        let mut harness = Harness::new_ui(|ui| {
            AudioPanel {
                commands: &mut commands,
                state: &mut state,
                snapshot: &snapshot,
            }
            .ui(ui);
        });
        harness.run();
        harness.snapshot("audio_panel_with_devices");
    }
}
