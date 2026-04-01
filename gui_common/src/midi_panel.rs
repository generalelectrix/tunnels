use eframe::egui;
use midi_harness::{list_ports, DeviceId, DeviceKind, MidiPortSpec, PortStatus, SlotStatus};

use crate::STATUS_COLORS;

/// Abstraction over project-specific command dispatch for MIDI panels.
pub trait MidiCommands {
    fn connect_port(&mut self, slot_name: &str, device_id: DeviceId, kind: DeviceKind);
    fn clear_device(&mut self, slot_name: &str);
    fn report_error(&mut self, error: impl std::fmt::Display);
}

pub struct MidiPanelState {
    input_ports: Vec<MidiPortSpec>,
    output_ports: Vec<MidiPortSpec>,
}

impl Default for MidiPanelState {
    fn default() -> Self {
        let (input_ports, output_ports) = list_ports().unwrap_or_default();
        Self {
            input_ports,
            output_ports,
        }
    }
}

impl MidiPanelState {
    fn refresh_ports(&mut self) {
        if let Ok((inputs, outputs)) = list_ports() {
            self.input_ports = inputs;
            self.output_ports = outputs;
        }
    }

    fn ports_for_kind(&self, kind: DeviceKind) -> &[MidiPortSpec] {
        match kind {
            DeviceKind::Input => &self.input_ports,
            DeviceKind::Output => &self.output_ports,
        }
    }
}

pub struct MidiPanel<'a, C: MidiCommands> {
    pub commands: &'a mut C,
    pub state: &'a mut MidiPanelState,
    pub slots: &'a [SlotStatus],
}

impl<C: MidiCommands> MidiPanel<'_, C> {
    pub fn ui(mut self, ui: &mut egui::Ui) {
        ui.heading("MIDI Devices");
        ui.separator();

        ui.horizontal(|ui| {
            if ui
                .button("\u{1f504}")
                .on_hover_text("Refresh port list")
                .clicked()
            {
                self.state.refresh_ports();
            }
        });
        ui.add_space(4.0);

        let slots = self.slots;

        if slots.is_empty() {
            ui.label("No MIDI slots configured.");
            return;
        }

        egui::Grid::new("midi_slots_grid")
            .num_columns(6)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Slot");
                ui.strong("Model");
                ui.strong("Input");
                ui.strong("Output");
                ui.label(""); // Auto button column
                ui.label(""); // Clear button column
                ui.end_row();

                for slot in slots {
                    ui.label(&slot.name);
                    ui.label(&slot.model);

                    self.port_combo(ui, &slot.name, &slot.input, DeviceKind::Input);
                    self.port_combo(ui, &slot.name, &slot.output, DeviceKind::Output);

                    if ui.button("Auto").clicked() {
                        self.auto_configure_slot(&slot.model, &slot.name);
                    }

                    if ui.button("Clear").clicked() {
                        self.commands.clear_device(&slot.name);
                    }

                    ui.end_row();
                }
            });
    }

    /// Render a combo box for selecting a MIDI port.
    fn port_combo(
        &mut self,
        ui: &mut egui::Ui,
        slot_name: &str,
        current: &PortStatus,
        kind: DeviceKind,
    ) {
        let kind_label = match kind {
            DeviceKind::Input => "input",
            DeviceKind::Output => "output",
        };
        let combo_id = format!("{slot_name}_{kind_label}");

        let (selected_text, text_color) = match current {
            PortStatus::Unassigned => ("Unassigned".to_string(), STATUS_COLORS.inactive),
            PortStatus::Connected { name, .. } => (name.clone(), STATUS_COLORS.active),
            PortStatus::Disconnected { name, .. } => {
                (format!("{name} (disconnected)"), STATUS_COLORS.error)
            }
        };

        let current_id = match current {
            PortStatus::Connected { id, .. } | PortStatus::Disconnected { id, .. } => Some(id),
            PortStatus::Unassigned => None,
        };

        let ports = self.state.ports_for_kind(kind);
        let commands = &mut self.commands;

        egui::ComboBox::from_id_salt(&combo_id)
            .selected_text(egui::RichText::new(&selected_text).color(text_color))
            .show_ui(ui, |ui| {
                if current_id.is_some() && ui.selectable_label(false, "Unassigned").clicked() {
                    commands.clear_device(slot_name);
                }

                for port in ports {
                    let is_selected = current_id == Some(&port.id);
                    if ui.selectable_label(is_selected, &port.name).clicked() && !is_selected {
                        commands.connect_port(slot_name, port.id.clone(), kind);
                    }
                }
            });
    }

    /// Try to auto-configure a slot by matching its model name against available ports.
    fn auto_configure_slot(&mut self, model: &str, slot_name: &str) {
        self.state.refresh_ports();

        let input_id = self
            .state
            .input_ports
            .iter()
            .find(|p| p.name == model)
            .map(|p| p.id.clone());
        let output_id = self
            .state
            .output_ports
            .iter()
            .find(|p| p.name == model)
            .map(|p| p.id.clone());

        if input_id.is_none() && output_id.is_none() {
            self.commands
                .report_error(format_args!("No matching MIDI port found for \"{model}\""));
            return;
        }

        if let Some(id) = input_id {
            self.commands.connect_port(slot_name, id, DeviceKind::Input);
        }
        if let Some(id) = output_id {
            self.commands
                .connect_port(slot_name, id, DeviceKind::Output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use midi_harness::{DeviceId, PortStatus, SlotStatus};

    struct NoOpMidiCommands;

    impl MidiCommands for NoOpMidiCommands {
        fn connect_port(&mut self, _: &str, _: DeviceId, _: DeviceKind) {}
        fn clear_device(&mut self, _: &str) {}
        fn report_error(&mut self, _: impl std::fmt::Display) {}
    }

    /// The default slots as they appear on startup -- all unassigned.
    fn default_slots() -> Vec<SlotStatus> {
        vec![
            SlotStatus {
                name: "APC-40".to_string(),
                model: "Akai APC40".to_string(),
                input: PortStatus::Unassigned,
                output: PortStatus::Unassigned,
            },
            SlotStatus {
                name: "TouchOSC".to_string(),
                model: "TouchOSC Bridge".to_string(),
                input: PortStatus::Unassigned,
                output: PortStatus::Unassigned,
            },
            SlotStatus {
                name: "Clock Wing".to_string(),
                model: "CMD MM-1".to_string(),
                input: PortStatus::Unassigned,
                output: PortStatus::Unassigned,
            },
        ]
    }

    /// Slots showing a mix of connection states.
    fn mixed_status_slots() -> Vec<SlotStatus> {
        vec![
            SlotStatus {
                name: "APC-40".to_string(),
                model: "Akai APC40".to_string(),
                input: PortStatus::Connected {
                    id: DeviceId("apc40-in".into()),
                    name: "Akai APC40".to_string(),
                },
                output: PortStatus::Connected {
                    id: DeviceId("apc40-out".into()),
                    name: "Akai APC40".to_string(),
                },
            },
            SlotStatus {
                name: "TouchOSC".to_string(),
                model: "TouchOSC Bridge".to_string(),
                input: PortStatus::Disconnected {
                    id: DeviceId("touchosc-in".into()),
                    name: "TouchOSC Bridge".to_string(),
                },
                output: PortStatus::Unassigned,
            },
            SlotStatus {
                name: "Clock Wing".to_string(),
                model: "CMD MM-1".to_string(),
                input: PortStatus::Unassigned,
                output: PortStatus::Unassigned,
            },
        ]
    }

    #[test]
    fn render_default_slots() {
        let slots = default_slots();
        let mut commands = NoOpMidiCommands;
        let mut harness = Harness::new_ui(|ui| {
            MidiPanel {
                commands: &mut commands,
                state: &mut MidiPanelState {
                    input_ports: vec![],
                    output_ports: vec![],
                },
                slots: &slots,
            }
            .ui(ui);
        });
        harness.run();
        harness.snapshot("midi_panel_default");
    }

    #[test]
    fn render_mixed_status() {
        let slots = mixed_status_slots();
        let mut commands = NoOpMidiCommands;
        let mut harness = Harness::new_ui(|ui| {
            MidiPanel {
                commands: &mut commands,
                state: &mut MidiPanelState {
                    input_ports: vec![],
                    output_ports: vec![],
                },
                slots: &slots,
            }
            .ui(ui);
        });
        harness.run();
        harness.snapshot("midi_panel_mixed_status");
    }
}
