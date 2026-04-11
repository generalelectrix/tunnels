use eframe::egui;
use std::time::Duration;

use crate::STATUS_COLORS;

/// Abstraction over project-specific command dispatch for audio panels.
pub trait AudioCommands {
    fn set_device(&mut self, device: Option<String>);
    fn set_filter_cutoff(&mut self, hz: f32);
    fn set_envelope_attack(&mut self, duration: Duration);
    fn set_envelope_release(&mut self, duration: Duration);
    fn set_output_smoothing(&mut self, duration: Duration);
    fn set_gain(&mut self, gain_linear: f64);
    fn set_auto_trim_enabled(&mut self, enabled: bool);
    fn set_active_band(&mut self, band: u32);
    fn set_norm_floor_halflife(&mut self, seconds: f32);
    fn set_norm_ceiling_halflife(&mut self, seconds: f32);
    fn set_norm_floor_mode(&mut self, mode: u32);
    fn set_norm_ceiling_mode(&mut self, mode: u32);
    fn toggle_monitor(&mut self);
    fn reset_parameters(&mut self);
    fn list_devices(&mut self) -> Vec<String>;
    fn report_error(&mut self, error: impl std::fmt::Display);
}

/// Read-only snapshot of audio parameter state for display.
/// Contains only values that change on user action (not streaming data).
#[derive(Debug, Clone)]
pub struct AudioSnapshot {
    pub device_name: String,
    pub filter_cutoff_hz: f32,
    pub envelope_attack: Duration,
    pub envelope_release: Duration,
    pub output_smoothing: Duration,
    pub gain_linear: f64,
    pub auto_trim_enabled: bool,
    pub active_band: u32,
    pub norm_floor_halflife: f32,
    pub norm_ceiling_halflife: f32,
    pub norm_floor_mode: u32,
    pub norm_ceiling_mode: u32,
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self {
            device_name: "Offline".to_string(),
            filter_cutoff_hz: 200.0,
            envelope_attack: Duration::from_millis(10),
            envelope_release: Duration::from_millis(50),
            output_smoothing: Duration::from_millis(8),
            gain_linear: 1.0,
            auto_trim_enabled: true,
            active_band: 0,
            norm_floor_halflife: 10.0,
            norm_ceiling_halflife: 5.0,
            norm_floor_mode: 0,
            norm_ceiling_mode: 1,
        }
    }
}

pub struct AudioPanelState {
    selected_audio: Option<usize>,
    audio_devices: Vec<String>,
}

impl AudioPanelState {
    pub fn new(devices: Vec<String>) -> Self {
        Self {
            selected_audio: None,
            audio_devices: devices,
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
        ui.heading("Audio Input");
        ui.separator();

        // Status indicator.
        let status_label = format!("Active: {}", self.snapshot.device_name);
        let status_color = if self.snapshot.device_name == "Offline" {
            STATUS_COLORS.inactive
        } else {
            STATUS_COLORS.active
        };
        ui.colored_label(status_color, &status_label);
        ui.add_space(8.0);

        self.device_selection(ui);
        ui.add_space(8.0);
        self.envelope_controls(ui);
        ui.add_space(8.0);
        self.band_and_agc_controls(ui);
    }

    fn device_selection(&mut self, ui: &mut egui::Ui) {
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
            self.commands.set_device(device_name);
        }
    }

    fn envelope_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Envelope");
        ui.add_space(4.0);

        egui::Grid::new("envelope_controls_grid")
            .min_col_width(200.0)
            .show(ui, |ui| {
                // Input gain (dB).
                ui.label("Input gain:");
                let mut gain_db = 20.0 * (self.snapshot.gain_linear as f32).log10();
                if ui
                    .add(egui::Slider::new(&mut gain_db, -20.0..=30.0).suffix(" dB"))
                    .changed()
                {
                    self.commands.set_gain(10.0_f64.powf(gain_db as f64 / 20.0));
                }
                ui.end_row();

                // Auto-trim toggle.
                ui.label("Auto trim:");
                let mut enabled = self.snapshot.auto_trim_enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    self.commands.set_auto_trim_enabled(enabled);
                }
                ui.end_row();

                // Filter cutoff.
                ui.label("Filter cutoff:");
                let mut cutoff = self.snapshot.filter_cutoff_hz;
                if ui
                    .add(
                        egui::Slider::new(&mut cutoff, 40.0..=1040.0)
                            .suffix(" Hz")
                            .logarithmic(true),
                    )
                    .changed()
                {
                    self.commands.set_filter_cutoff(cutoff);
                }
                ui.end_row();

                // Envelope attack.
                ui.label("Env attack:");
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

                // Envelope release.
                ui.label("Env release:");
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

                // Output smoothing.
                ui.label("Output smooth:");
                let mut smooth_ms = self.snapshot.output_smoothing.as_secs_f32() * 1000.0;
                if ui
                    .add(egui::Slider::new(&mut smooth_ms, 0.0..=50.0).suffix(" ms"))
                    .changed()
                {
                    self.commands
                        .set_output_smoothing(Duration::from_secs_f32(smooth_ms / 1000.0));
                }
                ui.end_row();
            });
    }

    fn band_and_agc_controls(&mut self, ui: &mut egui::Ui) {
        const BAND_LABELS: [&str; 8] = [
            "<187", "187-375", "375-750", "750-1.5k", "1.5-3k", "3-6k", "6-12k", "12-24k",
        ];

        ui.heading("Band / AGC");
        ui.add_space(4.0);

        egui::Grid::new("band_agc_grid")
            .min_col_width(200.0)
            .show(ui, |ui| {
                // Band selector.
                ui.label("Active band:");
                let mut band = self.snapshot.active_band;
                let selected_text = BAND_LABELS.get(band as usize).copied().unwrap_or("<187");
                egui::ComboBox::from_id_salt("active_band")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for (i, label) in BAND_LABELS.iter().enumerate() {
                            ui.selectable_value(&mut band, i as u32, *label);
                        }
                    });
                if band != self.snapshot.active_band {
                    self.commands.set_active_band(band);
                }
                ui.end_row();

                // Floor half-life.
                ui.label("Floor half-life:");
                let mut floor_hl = self.snapshot.norm_floor_halflife;
                if ui
                    .add(
                        egui::Slider::new(&mut floor_hl, 0.5..=30.0)
                            .suffix(" s")
                            .logarithmic(true),
                    )
                    .changed()
                {
                    self.commands.set_norm_floor_halflife(floor_hl);
                }
                ui.end_row();

                // Floor mode.
                ui.label("Floor mode:");
                ui.horizontal(|ui| {
                    let mut mode = self.snapshot.norm_floor_mode;
                    if ui.selectable_label(mode == 0, "Avg").clicked() {
                        mode = 0;
                    }
                    if ui.selectable_label(mode == 1, "Min").clicked() {
                        mode = 1;
                    }
                    if mode != self.snapshot.norm_floor_mode {
                        self.commands.set_norm_floor_mode(mode);
                    }
                });
                ui.end_row();

                // Ceiling half-life.
                ui.label("Ceil half-life:");
                let mut ceil_hl = self.snapshot.norm_ceiling_halflife;
                if ui
                    .add(
                        egui::Slider::new(&mut ceil_hl, 0.5..=15.0)
                            .suffix(" s")
                            .logarithmic(true),
                    )
                    .changed()
                {
                    self.commands.set_norm_ceiling_halflife(ceil_hl);
                }
                ui.end_row();

                // Ceiling mode.
                ui.label("Ceil mode:");
                ui.horizontal(|ui| {
                    let mut mode = self.snapshot.norm_ceiling_mode;
                    if ui.selectable_label(mode == 0, "Avg").clicked() {
                        mode = 0;
                    }
                    if ui.selectable_label(mode == 1, "Max").clicked() {
                        mode = 1;
                    }
                    if mode != self.snapshot.norm_ceiling_mode {
                        self.commands.set_norm_ceiling_mode(mode);
                    }
                });
                ui.end_row();
            });

        ui.add_space(4.0);
        if ui.button("Reset All").clicked() {
            self.commands.reset_parameters();
        }
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
        fn set_filter_cutoff(&mut self, _hz: f32) {}
        fn set_envelope_attack(&mut self, _duration: Duration) {}
        fn set_envelope_release(&mut self, _duration: Duration) {}
        fn set_output_smoothing(&mut self, _duration: Duration) {}
        fn set_gain(&mut self, _gain_linear: f64) {}
        fn set_auto_trim_enabled(&mut self, _enabled: bool) {}
        fn set_active_band(&mut self, _band: u32) {}
        fn set_norm_floor_halflife(&mut self, _seconds: f32) {}
        fn set_norm_ceiling_halflife(&mut self, _seconds: f32) {}
        fn set_norm_floor_mode(&mut self, _mode: u32) {}
        fn set_norm_ceiling_mode(&mut self, _mode: u32) {}
        fn toggle_monitor(&mut self) {}
        fn reset_parameters(&mut self) {}
        fn list_devices(&mut self) -> Vec<String> {
            self.devices.clone()
        }
        fn report_error(&mut self, _error: impl std::fmt::Display) {}
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

    #[test]
    fn render_auto_trim_disabled() {
        use egui_kittest::Harness;
        let mut commands = MockAudioCommands::new(vec![]);
        let mut state = AudioPanelState::new(vec![]);
        let snapshot = AudioSnapshot {
            device_name: "Scarlett 2i2 USB".to_string(),
            auto_trim_enabled: false,
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
        harness.snapshot("audio_panel_auto_trim_disabled");
    }
}
