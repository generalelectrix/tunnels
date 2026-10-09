use eframe::egui::{self, Color32};
use std::sync::Weak;
use std::time::Duration;
pub use tunnels_audio::AudioSnapshot;
use tunnels_audio::roles::Role;
use tunnels_audio::time::HalfLife;
use tunnels_audio::{InputMeter, OFFLINE_DEVICE_NAME};

use crate::STATUS_COLORS;

/// The resolution the input trim is displayed at, in dB.
const TRIM_DISPLAY_STEP_DB: f32 = 0.5;

/// How often a GUI showing the input meter redraws to follow it.
pub const METER_REFRESH: Duration = Duration::from_millis(30);

/// The clip indicator's color while dark.
const CLIP_LED_DARK: Color32 = Color32::from_gray(48);

/// Abstraction over project-specific command dispatch for audio panels.
pub trait AudioCommands {
    fn set_device(&mut self, device: Option<String>);
    fn set_envelope_attack(&mut self, duration: Duration);
    fn set_envelope_release(&mut self, duration: Duration);
    fn set_output_smoothing(&mut self, duration: Duration);
    fn set_active_role(&mut self, role: Role);
    fn set_norm_floor_halflife(&mut self, halflife: HalfLife);
    fn set_norm_ceiling_halflife(&mut self, halflife: HalfLife);
    fn reset_parameters(&mut self);
    fn list_devices(&mut self) -> Vec<String>;
}

pub struct AudioPanelState {
    selected_audio: Option<usize>,
    audio_devices: Vec<String>,
    /// The input's meter, readable only while the input it belongs to is
    /// open.
    input_meter: Weak<InputMeter>,
}

impl AudioPanelState {
    pub fn new(devices: Vec<String>) -> Self {
        Self {
            selected_audio: None,
            audio_devices: devices,
            input_meter: Weak::new(),
        }
    }

    /// Read the input trim and clip indicator from `meter` for as long as it
    /// upgrades.
    pub fn set_input_meter(&mut self, meter: Weak<InputMeter>) {
        self.input_meter = meter;
    }

    /// Whether the input meter's clip indicator is lit, or `None` when no
    /// live meter is attached.
    pub fn input_clipping(&self) -> Option<bool> {
        self.input_meter.upgrade().map(|m| m.clip_lit())
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
            // The envelope the show follows.
            ui.label("Follow:");
            if let Some(role) = follow_selector(ui, self.snapshot.active_role) {
                self.commands.set_active_role(role);
            }
            ui.end_row();

            ui.label("Input trim:").on_hover_text(
                "Gain the input is automatically trimmed by, so its peaks sit near full scale.",
            );
            ui.label(match self.state.input_meter.upgrade() {
                Some(meter) => format!("{:+.1} dB", displayed_trim_db(meter.trim_db())),
                None => "--".to_string(),
            });
            ui.end_row();
        });
    }

    fn envelope_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Envelope").on_hover_text(
            "Shapes the level roles, Bass and Shimmer. Kick and Hats are hits, with timing of their own.",
        );
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
            let mut floor_hl_s = self.snapshot.norm_floor_halflife.get().as_secs_f32();
            if ui
                .add(
                    egui::Slider::new(&mut floor_hl_s, 0.5..=30.0)
                        .suffix(" s")
                        .logarithmic(true),
                )
                .changed()
                && let Some(halflife) = HalfLife::try_from_secs_f32(floor_hl_s)
            {
                self.commands.set_norm_floor_halflife(halflife);
            }
            ui.end_row();

            ui.label("Peak Memory:");
            let mut ceil_hl_s = self.snapshot.norm_ceiling_halflife.get().as_secs_f32();
            if ui
                .add(
                    egui::Slider::new(&mut ceil_hl_s, 0.5..=60.0)
                        .suffix(" s")
                        .logarithmic(true),
                )
                .changed()
                && let Some(halflife) = HalfLife::try_from_secs_f32(ceil_hl_s)
            {
                self.commands.set_norm_ceiling_halflife(halflife);
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

/// A trim in dB to the nearest [`TRIM_DISPLAY_STEP_DB`], never negative zero.
fn displayed_trim_db(trim_db: f32) -> f32 {
    (trim_db / TRIM_DISPLAY_STEP_DB).round() * TRIM_DISPLAY_STEP_DB + 0.0
}

/// Render a selector for the audio role to follow, showing `active_role`.
///
/// Returns the role chosen this frame, if it differs from `active_role`. The
/// caller labels the selector to suit its layout.
pub fn follow_selector(ui: &mut egui::Ui, active_role: Role) -> Option<Role> {
    let mut role = active_role;
    egui::ComboBox::from_id_salt("active_role")
        .selected_text(role.label())
        .show_ui(ui, |ui| {
            for r in Role::ALL {
                ui.selectable_value(&mut role, r, r.label());
            }
        });
    (role != active_role).then_some(role)
}

/// Render the tab that holds the audio panel in a tab bar, labelled `label`,
/// returning `true` if it was clicked this frame. With `clip` present, a round
/// input-clipping LED sits inside the tab to the right of its label: red while
/// `clip` is `Some(true)`, dark otherwise.
pub fn audio_tab(ui: &mut egui::Ui, label: &str, selected: bool, clip: Option<bool>) -> bool {
    let Some(lit) = clip else {
        return ui.selectable_label(selected, label).clicked();
    };
    let led_atom = egui::Id::new("clip_led");
    let diameter = ui.spacing().interact_size.y * 0.6;
    let tab = egui::Button::selectable(
        selected,
        (
            label,
            egui::Atom::custom(led_atom, egui::Vec2::splat(diameter)),
        ),
    )
    .atom_ui(ui);
    if let Some(rect) = tab.rect(led_atom) {
        let color = if lit {
            STATUS_COLORS.error
        } else {
            CLIP_LED_DARK
        };
        ui.painter()
            .circle_filled(rect.center(), diameter / 2.0, color);
        ui.interact(rect, tab.response.id.with(led_atom), egui::Sense::hover())
            .on_hover_text("Input clipping");
    }
    tab.response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        fn set_active_role(&mut self, _role: Role) {}
        fn set_norm_floor_halflife(&mut self, _halflife: HalfLife) {}
        fn set_norm_ceiling_halflife(&mut self, _halflife: HalfLife) {}
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
        let meter = Arc::new(InputMeter::default());
        meter.set(3.4, true);
        state.set_input_meter(Arc::downgrade(&meter));
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
    fn render_audio_tab_states() {
        use egui_kittest::Harness;
        // No meter (plain tab), meter dark, meter lit.
        let mut harness = Harness::new_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = audio_tab(ui, "Audio", false, None);
                let _ = audio_tab(ui, "Audio", false, Some(false));
                let _ = audio_tab(ui, "Audio", true, Some(true));
            });
        });
        harness.run();
        harness.snapshot("audio_tab_clip_led");
    }

    /// Render the panel for an open device and assert that `trim_text` is
    /// shown in the input trim row.
    fn assert_trim_row_reads(state: &mut AudioPanelState, trim_text: &str) {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut commands = MockAudioCommands::new(vec![]);
        let snapshot = AudioSnapshot {
            device_name: "Scarlett 2i2 USB".to_string(),
            ..default_snapshot()
        };
        let mut harness = Harness::new_ui(|ui| {
            AudioPanel {
                commands: &mut commands,
                state: &mut *state,
                snapshot: &snapshot,
            }
            .ui(ui);
        });
        harness.run();
        harness.get_by_label(trim_text);
    }

    #[test]
    fn meter_is_read_only_while_it_lives() {
        let mut state = AudioPanelState::new(vec![]);
        assert_eq!(state.input_clipping(), None, "no meter attached");
        assert_trim_row_reads(&mut state, "--");

        let meter = Arc::new(InputMeter::default());
        meter.set(3.4, true);
        state.set_input_meter(Arc::downgrade(&meter));
        assert_eq!(state.input_clipping(), Some(true), "a live, lit meter");
        assert_trim_row_reads(&mut state, "+3.5 dB");

        drop(meter);
        assert_eq!(state.input_clipping(), None, "the meter is gone");
        assert_trim_row_reads(&mut state, "--");
    }

    #[test]
    fn trim_is_displayed_to_the_nearest_half_db() {
        assert_eq!(displayed_trim_db(3.1), 3.0);
        assert_eq!(displayed_trim_db(3.4), 3.5);
        assert_eq!(displayed_trim_db(-9.8), -10.0);
        assert_eq!(
            format!("{:+.1}", displayed_trim_db(-0.2)),
            "+0.0",
            "a trim that rounds to zero shows no sign of having been negative"
        );
    }
}
