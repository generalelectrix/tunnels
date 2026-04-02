// Re-export the shared panel types.
pub use gui_common::midi_panel::{MidiCommands, MidiPanel, MidiPanelState};

use midi_harness::{DeviceId, DeviceKind};
use tunnels::control::MetaCommand;

use crate::ui_util::GuiContext;

impl<App: 'static> MidiCommands for GuiContext<'_, App> {
    fn connect_port(&mut self, slot_name: &str, device_id: DeviceId, kind: DeviceKind) {
        let _ = self.send_command(MetaCommand::ConnectMidiPort {
            slot_name: slot_name.to_string(),
            device_id,
            kind,
        });
    }

    fn clear_device(&mut self, slot_name: &str) {
        let _ = self.send_command(MetaCommand::ClearMidiDevice {
            slot_name: slot_name.to_string(),
        });
    }

    fn report_error(&mut self, error: impl std::fmt::Display) {
        self.modal.show("Error", error.to_string());
    }
}
