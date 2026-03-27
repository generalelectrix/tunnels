use gui_common::MessageModal;
use tunnels::control::{CommandClient, MetaCommand};

/// Shared rendering context for GUI panels.
///
/// Bundles the dependencies common to all panel renderers so they don't
/// need to be threaded through every method call.
pub(crate) struct GuiContext<'a> {
    pub modal: &'a mut MessageModal,
    pub client: &'a CommandClient,
}

impl GuiContext<'_> {
    pub fn report_error(&mut self, error: impl std::fmt::Display) {
        self.modal.show("Error", error.to_string());
    }

    pub fn send_command(&mut self, cmd: MetaCommand) -> Result<(), anyhow::Error> {
        self.client.send_command(cmd).inspect_err(|e| {
            self.modal.show("Error", e.to_string());
        })
    }
}
