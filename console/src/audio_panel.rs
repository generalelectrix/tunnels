use std::time::Duration;

use gui_common::audio_panel::{
    AudioCommands, AudioPanel as SharedAudioPanel, AudioPanelState as SharedAudioPanelState,
    AudioSnapshot,
};
use tunnels::audio::AudioDevice;
use tunnels::control::MetaCommand;

use crate::ui_util::GuiContext;

/// Console-specific audio panel state that holds the full device handles.
pub struct AudioPanelState {
    pub inner: SharedAudioPanelState,
    devices: Vec<AudioDevice>,
}

impl AudioPanelState {
    pub fn new(devices: Vec<AudioDevice>) -> Self {
        let names: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
        Self {
            inner: SharedAudioPanelState::new(names),
            devices,
        }
    }

    pub fn sync_from_device_name(&mut self, device_name: &str) {
        self.inner.sync_from_device_name(device_name);
    }
}

/// Adapter that implements AudioCommands using the console's GuiContext
/// and the stored device list.
struct ConsoleAudioCommands<'a> {
    ctx: GuiContext<'a>,
    devices: &'a mut Vec<AudioDevice>,
}

impl AudioCommands for ConsoleAudioCommands<'_> {
    fn set_device(&mut self, device_index: Option<usize>) {
        let device = device_index.and_then(|i| self.devices.get(i).cloned());
        let _ = self.ctx.send_command(MetaCommand::SetAudioDevice(device));
    }

    fn set_filter_cutoff(&mut self, hz: f32) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::FilterCutoff(hz)),
        ));
    }

    fn set_envelope_attack(&mut self, duration: Duration) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::EnvelopeAttack(
                duration,
            )),
        ));
    }

    fn set_envelope_release(&mut self, duration: Duration) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::EnvelopeRelease(
                duration,
            )),
        ));
    }

    fn set_output_smoothing(&mut self, duration: Duration) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::OutputSmoothing(
                duration,
            )),
        ));
    }

    fn set_gain(&mut self, gain_linear: f64) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::InputGain(
                gain_linear,
            )),
        ));
    }

    fn set_auto_trim_enabled(&mut self, enabled: bool) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::AutoTrimEnabled(
                enabled,
            )),
        ));
    }

    fn set_active_band(&mut self, band: u32) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::ActiveBand(band)),
        ));
    }

    fn set_norm_floor_halflife(&mut self, seconds: f32) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::NormFloorHalflife(
                seconds,
            )),
        ));
    }

    fn set_norm_ceiling_halflife(&mut self, seconds: f32) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::NormCeilingHalflife(
                seconds,
            )),
        ));
    }

    fn set_norm_floor_mode(&mut self, mode: u32) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::NormFloorMode(mode)),
        ));
    }

    fn set_norm_ceiling_mode(&mut self, mode: u32) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::Set(tunnels::audio::StateChange::NormCeilingMode(mode)),
        ));
    }

    fn toggle_monitor(&mut self) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::ToggleMonitor,
        ));
    }

    fn reset_parameters(&mut self) {
        let _ = self.ctx.send_command(MetaCommand::AudioControl(
            tunnels::audio::ControlMessage::ResetParameters,
        ));
    }

    fn list_devices(&mut self) -> Vec<String> {
        match tunnels::audio::AudioInput::devices() {
            Ok(new_devices) => {
                let names: Vec<String> = new_devices.iter().map(|d| d.name.clone()).collect();
                self.devices.clear();
                self.devices.extend(new_devices);
                names
            }
            Err(e) => {
                self.ctx
                    .report_error(format_args!("Failed to refresh audio devices: {e}"));
                vec![]
            }
        }
    }

    fn report_error(&mut self, error: impl std::fmt::Display) {
        self.ctx.report_error(error);
    }
}

/// Convenience wrapper to render the audio panel using the console's infrastructure.
pub(crate) fn render_audio_panel(
    ui: &mut eframe::egui::Ui,
    ctx: GuiContext<'_>,
    state: &mut AudioPanelState,
    snapshot: &AudioSnapshot,
) {
    let mut commands = ConsoleAudioCommands {
        ctx,
        devices: &mut state.devices,
    };
    SharedAudioPanel {
        commands: &mut commands,
        state: &mut state.inner,
        snapshot,
    }
    .ui(ui);
}
