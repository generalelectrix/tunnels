//! egui-based panel for administering tunnel clients.

use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use client_lib::admin::Administrator;
use client_lib::config::ClientConfig;
use client_lib::transform::{Transform, TransformDirection};

/// Abstraction over the network admin operations so we can mock them in tests.
pub trait AdminService: Send + Sync {
    fn clients(&self) -> Vec<String>;
    fn run_with_config(&self, client: &str, config: ClientConfig) -> anyhow::Result<String>;
}

impl AdminService for Administrator {
    fn clients(&self) -> Vec<String> {
        self.clients()
    }
    fn run_with_config(&self, client: &str, config: ClientConfig) -> anyhow::Result<String> {
        self.run_with_config(client, config)
    }
}

/// The selected target for configuration: either a local monitor or a remote client.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Monitor,
    RemoteClient(String),
}

/// State of an in-progress or completed config send / monitor launch.
enum ConfigSendState {
    Sending { label: String },
    Success { label: String, message: String },
    Error { label: String, message: String },
}

/// Pre-baked resolution presets.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ResolutionPreset {
    P1080,
    P720,
    Wuxga,
    SxgaPlus,
    Custom,
}

impl ResolutionPreset {
    fn label(&self) -> &'static str {
        match self {
            Self::P1080 => "1080p (1920x1080)",
            Self::P720 => "720p (1280x720)",
            Self::Wuxga => "WUXGA (1920x1200)",
            Self::SxgaPlus => "SXGA+ (1400x1050)",
            Self::Custom => "Custom",
        }
    }

    fn resolution(&self) -> Option<(u32, u32)> {
        match self {
            Self::P1080 => Some((1920, 1080)),
            Self::P720 => Some((1280, 720)),
            Self::Wuxga => Some((1920, 1200)),
            Self::SxgaPlus => Some((1400, 1050)),
            Self::Custom => None,
        }
    }

    const ALL: [Self; 5] = [
        Self::P1080,
        Self::P720,
        Self::Wuxga,
        Self::SxgaPlus,
        Self::Custom,
    ];
}

/// Find the tunnelclient binary, either as a sibling of the current executable or on PATH.
fn tunnelclient_path() -> Result<std::path::PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.parent()
            .unwrap_or(std::path::Path::new("."))
            .join("tunnelclient");
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    // Fall back to PATH.
    Ok(std::path::PathBuf::from("tunnelclient"))
}

pub struct AdminPanelState {
    admin_service: Arc<dyn AdminService>,
    hostname: String,

    // UI state
    selected_target: Option<Target>,
    video_channel: u64,
    resolution_preset: ResolutionPreset,
    custom_width: String,
    custom_height: String,
    half_size: bool,
    fullscreen: bool,
    flip_horizontal: bool,
    capture_mouse: bool,

    // Async config send / monitor launch
    config_send_state: Arc<Mutex<Option<ConfigSendState>>>,
}

impl AdminPanelState {
    pub fn new(admin_service: Arc<dyn AdminService>, hostname: String) -> Self {
        let (w, h) = ResolutionPreset::P1080.resolution().unwrap();
        Self {
            admin_service,
            hostname,
            selected_target: Some(Target::Monitor),
            video_channel: 0,
            resolution_preset: ResolutionPreset::P1080,
            custom_width: (w / 2).to_string(),
            custom_height: (h / 2).to_string(),
            half_size: true,
            fullscreen: false,
            flip_horizontal: false,
            capture_mouse: false,
            config_send_state: Arc::new(Mutex::new(None)),
        }
    }

    /// Update the displayed width/height fields to reflect the current preset and half_size state.
    fn update_displayed_resolution(&mut self) {
        if let Some((w, h)) = self.resolution_preset.resolution() {
            if self.half_size {
                self.custom_width = (w / 2).to_string();
                self.custom_height = (h / 2).to_string();
            } else {
                self.custom_width = w.to_string();
                self.custom_height = h.to_string();
            }
        }
    }

    /// Resolve the current resolution from preset or custom fields.
    /// For presets, applies half_size scaling if enabled.
    fn resolve_resolution(&self) -> Result<(u32, u32), String> {
        if let Some((w, h)) = self.resolution_preset.resolution() {
            if self.half_size {
                Ok((w / 2, h / 2))
            } else {
                Ok((w, h))
            }
        } else {
            let w: u32 = self
                .custom_width
                .parse()
                .map_err(|_| format!("Invalid width: '{}'", self.custom_width))?;
            let h: u32 = self
                .custom_height
                .parse()
                .map_err(|_| format!("Invalid height: '{}'", self.custom_height))?;
            if w == 0 || h == 0 {
                return Err("Width and height must be greater than 0".to_string());
            }
            Ok((w, h))
        }
    }

    /// Build a ClientConfig from the current UI state.
    fn build_config(&self) -> Result<ClientConfig, String> {
        let resolution = self.resolve_resolution()?;
        let transformation = if self.flip_horizontal {
            Some(Transform::Flip(TransformDirection::Horizontal))
        } else {
            None
        };
        Ok(ClientConfig::new(
            self.video_channel,
            self.hostname.clone(),
            resolution,
            self.fullscreen,
            self.capture_mouse,
            transformation,
            false,
        ))
    }

    /// Switch to a new target, applying dynamic defaults.
    fn select_target(&mut self, target: Target) {
        if self.selected_target.as_ref() == Some(&target) {
            return;
        }
        match &target {
            Target::Monitor => {
                self.half_size = true;
                self.fullscreen = false;
                self.capture_mouse = false;
            }
            Target::RemoteClient(_) => {
                self.half_size = false;
                self.fullscreen = true;
                self.capture_mouse = true;
            }
        }
        self.update_displayed_resolution();
        self.selected_target = Some(target);
    }

    /// Perform the action for the current target selection.
    fn perform_action(&self) {
        match &self.selected_target {
            Some(Target::Monitor) => self.launch_monitor(),
            Some(Target::RemoteClient(_)) => self.send_config(),
            None => {}
        }
    }

    /// Launch a local monitor subprocess.
    fn launch_monitor(&self) {
        let config = match self.build_config() {
            Ok(cfg) => cfg,
            Err(e) => {
                *self.config_send_state.lock().unwrap() = Some(ConfigSendState::Error {
                    label: "monitor".to_string(),
                    message: e,
                });
                return;
            }
        };

        *self.config_send_state.lock().unwrap() = Some(ConfigSendState::Sending {
            label: "monitor".to_string(),
        });

        let state = self.config_send_state.clone();
        thread::spawn(move || {
            let result = (|| -> Result<String, String> {
                let serialized = rmp_serde::to_vec(&config)
                    .map_err(|e| format!("Failed to serialize config: {e}"))?;

                let exe = tunnelclient_path()?;

                let mut child = Command::new(exe)
                    .arg("monitor")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("Failed to spawn monitor: {e}"))?;

                // Write config to child stdin and close it.
                {
                    let mut stdin = child.stdin.take().unwrap();
                    stdin
                        .write_all(&serialized)
                        .map_err(|e| format!("Failed to write config to monitor: {e}"))?;
                }

                // Read the status line from child stdout.
                let stdout = child.stdout.take().unwrap();
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .map_err(|e| format!("Failed to read monitor status: {e}"))?;

                let line = line.trim();
                if line == "OK" {
                    Ok("Monitor launched successfully.".to_string())
                } else if let Some(err) = line.strip_prefix("ERROR: ") {
                    Err(err.to_string())
                } else if line.is_empty() {
                    Err("Monitor process exited without status.".to_string())
                } else {
                    Err(format!("Unexpected monitor response: {line}"))
                }
            })();

            let mut guard = state.lock().unwrap();
            *guard = Some(match result {
                Ok(msg) => ConfigSendState::Success {
                    label: "monitor".to_string(),
                    message: msg,
                },
                Err(e) => ConfigSendState::Error {
                    label: "monitor".to_string(),
                    message: e,
                },
            });
        });
    }

    /// Send configuration to the selected remote client on a background thread.
    fn send_config(&self) {
        let client_name = match &self.selected_target {
            Some(Target::RemoteClient(name)) => name.clone(),
            _ => return,
        };

        let config = match self.build_config() {
            Ok(cfg) => cfg,
            Err(e) => {
                *self.config_send_state.lock().unwrap() = Some(ConfigSendState::Error {
                    label: client_name,
                    message: e,
                });
                return;
            }
        };

        *self.config_send_state.lock().unwrap() = Some(ConfigSendState::Sending {
            label: client_name.clone(),
        });

        let admin = self.admin_service.clone();
        let state = self.config_send_state.clone();
        let name = client_name.clone();
        thread::spawn(move || {
            let result = admin.run_with_config(&name, config);
            let mut guard = state.lock().unwrap();
            *guard = Some(match result {
                Ok(msg) => ConfigSendState::Success {
                    label: name,
                    message: msg,
                },
                Err(e) => ConfigSendState::Error {
                    label: name,
                    message: e.to_string(),
                },
            });
        });
    }

    /// Draw the modal overlay if a config send is in progress or completed.
    fn draw_modal(&self, ctx: &egui::Context) {
        let state = self.config_send_state.lock().unwrap();
        if state.is_none() {
            return;
        }
        // Drop the lock before showing the modal, since button clicks need to acquire it.
        let (title, body, is_pending) = match state.as_ref().unwrap() {
            ConfigSendState::Sending { label } => (
                "Working...".to_string(),
                format!("Sending config to {}...", label),
                true,
            ),
            ConfigSendState::Success { label, message } => (
                "Success".to_string(),
                format!("{}:\n{}", label, message),
                false,
            ),
            ConfigSendState::Error { label, message } => (
                "Error".to_string(),
                format!("Failed ({}):\n{}", label, message),
                false,
            ),
        };
        drop(state);

        egui::Window::new(&title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&body);
                ui.add_space(8.0);
                if is_pending {
                    ui.spinner();
                    ui.add_space(4.0);
                    if ui.button("Stop Waiting").clicked() {
                        *self.config_send_state.lock().unwrap() = None;
                    }
                } else if ui.button("Ok").clicked() {
                    *self.config_send_state.lock().unwrap() = None;
                }
            });
    }

    /// Render the full admin UI. Called by `ConfigApp::update` and by test harnesses.
    pub fn render(&mut self, ctx: &egui::Context, clients: &[String]) {
        let is_sending = self.config_send_state.lock().unwrap().is_some();

        if is_sending {
            ctx.request_repaint();
        }

        // Invalidate selection if the selected remote client has disappeared.
        if let Some(Target::RemoteClient(ref name)) = self.selected_target {
            if !clients.iter().any(|c| c == name) {
                self.selected_target = None;
            }
        }

        // Use a consistent frame for both panels so headings and separators align.
        let panel_frame = egui::Frame::central_panel(&ctx.style());

        // Left panel: target list
        let side_panel_width = 180.0;
        egui::SidePanel::left("clients_panel")
            .min_width(side_panel_width)
            .max_width(side_panel_width)
            .frame(panel_frame)
            .show(ctx, |ui| {
                ui.heading("Targets");
                ui.separator();

                // Monitor is always first.
                let is_monitor = self.selected_target == Some(Target::Monitor);
                if ui.selectable_label(is_monitor, "monitor").clicked() {
                    self.select_target(Target::Monitor);
                }

                if !clients.is_empty() {
                    ui.separator();
                }

                // Discovered remote clients.
                for name in clients {
                    let target = Target::RemoteClient(name.clone());
                    let selected = self.selected_target.as_ref() == Some(&target);
                    if ui.selectable_label(selected, name).clicked() {
                        self.select_target(target);
                    }
                }
            });

        // Central panel: configuration form (only shown when a target is selected)
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(target) = self.selected_target.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a target to configure.");
                });
                return;
            };

            ui.heading("Configuration");
            ui.separator();

            // Video channel slider
            ui.horizontal(|ui| {
                ui.label("Video Channel:");
                let mut channel = self.video_channel as i32;
                if ui.add(egui::Slider::new(&mut channel, 0..=7)).changed() {
                    self.video_channel = channel as u64;
                }
            });

            ui.add_space(8.0);

            // Resolution preset combo box
            let current_label = self.resolution_preset.label();
            egui::ComboBox::from_label("Resolution")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for preset in &ResolutionPreset::ALL {
                        if ui
                            .selectable_value(&mut self.resolution_preset, *preset, preset.label())
                            .changed()
                        {
                            self.update_displayed_resolution();
                        }
                    }
                });

            // Width and height fields
            let is_custom = self.resolution_preset == ResolutionPreset::Custom;
            ui.horizontal(|ui| {
                ui.label("Width:");
                ui.add_enabled(
                    is_custom,
                    egui::TextEdit::singleline(&mut self.custom_width).desired_width(60.0),
                );
                ui.label("Height:");
                ui.add_enabled(
                    is_custom,
                    egui::TextEdit::singleline(&mut self.custom_height).desired_width(60.0),
                );
            });

            // Half Size checkbox (only enabled for presets, grayed out for Custom)
            let half_size_before = self.half_size;
            ui.add_enabled_ui(!is_custom, |ui| {
                ui.checkbox(&mut self.half_size, "Half Size");
            });
            if self.half_size != half_size_before {
                self.update_displayed_resolution();
            }

            ui.add_space(8.0);

            // Checkboxes
            ui.checkbox(&mut self.fullscreen, "Fullscreen");
            ui.checkbox(&mut self.flip_horizontal, "Flip Horizontal");
            ui.checkbox(&mut self.capture_mouse, "Capture Mouse");

            ui.add_space(16.0);

            // Action button -- label depends on target type
            let button_label = match &target {
                Target::Monitor => "Launch Monitor",
                Target::RemoteClient(_) => "Send Configuration",
            };
            if ui
                .add_enabled(!is_sending, egui::Button::new(button_label))
                .clicked()
            {
                self.perform_action();
            }
        });

        // Modal overlay
        self.draw_modal(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    struct MockAdminService {
        clients: Vec<String>,
    }

    impl AdminService for MockAdminService {
        fn clients(&self) -> Vec<String> {
            self.clients.clone()
        }
        fn run_with_config(&self, _client: &str, _config: ClientConfig) -> anyhow::Result<String> {
            Ok("Mock: configuration accepted.".to_string())
        }
    }

    impl AdminPanelState {
        fn test_new(clients: Vec<String>) -> Self {
            let admin: Arc<dyn AdminService> = Arc::new(MockAdminService {
                clients: clients.clone(),
            });
            AdminPanelState::new(admin, "test-host".to_string())
        }
    }

    fn test_harness(clients: Vec<String>) -> Harness<'static, AdminPanelState> {
        let clients_for_render = clients.clone();
        let harness = Harness::new_ui_state(
            move |ui, app: &mut AdminPanelState| {
                app.render(ui.ctx(), &clients_for_render);
            },
            AdminPanelState::test_new(clients),
        );
        stage_theme::apply(&harness.ctx);
        harness
    }

    // --- Default state tests ---

    #[test]
    fn default_state_selects_monitor() {
        let app = AdminPanelState::test_new(vec![]);
        assert_eq!(app.selected_target, Some(Target::Monitor));
        assert!(!app.fullscreen);
        assert!(!app.capture_mouse);
        assert!(app.half_size);
    }

    #[test]
    fn default_resolution_is_half_1080p() {
        let app = AdminPanelState::test_new(vec![]);
        assert_eq!(app.custom_width, "960");
        assert_eq!(app.custom_height, "540");
    }

    // --- Button text tests ---

    #[test]
    fn monitor_shows_launch_button() {
        let harness = test_harness(vec![]);
        assert!(harness.query_by_label("Launch Monitor").is_some());
    }

    #[test]
    fn remote_client_shows_send_button() {
        let clients = vec!["projector-1".to_string()];
        let mut harness = test_harness(clients);
        harness
            .state_mut()
            .select_target(Target::RemoteClient("projector-1".to_string()));
        harness.run();
        assert!(harness.query_by_label("Send Configuration").is_some());
    }

    // --- Dynamic defaults tests ---

    #[test]
    fn switching_to_remote_sets_defaults() {
        let mut app = AdminPanelState::test_new(vec!["client-a".to_string()]);
        app.select_target(Target::RemoteClient("client-a".to_string()));
        assert!(app.fullscreen);
        assert!(app.capture_mouse);
        assert!(!app.half_size);
        assert_eq!(app.custom_width, "1920");
        assert_eq!(app.custom_height, "1080");
    }

    #[test]
    fn switching_to_monitor_sets_defaults() {
        let mut app = AdminPanelState::test_new(vec!["client-a".to_string()]);
        // First switch to remote, then back to monitor.
        app.select_target(Target::RemoteClient("client-a".to_string()));
        app.select_target(Target::Monitor);
        assert!(!app.fullscreen);
        assert!(!app.capture_mouse);
        assert!(app.half_size);
        assert_eq!(app.custom_width, "960");
        assert_eq!(app.custom_height, "540");
    }

    // --- Half size tests ---

    #[test]
    fn half_size_halves_displayed_resolution() {
        let mut app = AdminPanelState::test_new(vec![]);
        app.half_size = true;
        app.update_displayed_resolution();
        assert_eq!(app.custom_width, "960");
        assert_eq!(app.custom_height, "540");
    }

    #[test]
    fn half_size_off_shows_full_resolution() {
        let mut app = AdminPanelState::test_new(vec![]);
        app.half_size = false;
        app.update_displayed_resolution();
        assert_eq!(app.custom_width, "1920");
        assert_eq!(app.custom_height, "1080");
    }

    #[test]
    fn half_size_halves_resolved_resolution() {
        let mut app = AdminPanelState::test_new(vec![]);
        app.half_size = true;
        assert_eq!(app.resolve_resolution().unwrap(), (960, 540));
    }

    #[test]
    fn half_size_ignored_for_custom() {
        let mut app = AdminPanelState::test_new(vec![]);
        app.resolution_preset = ResolutionPreset::Custom;
        app.custom_width = "800".to_string();
        app.custom_height = "600".to_string();
        app.half_size = true;
        // Half size should NOT halve custom values.
        assert_eq!(app.resolve_resolution().unwrap(), (800, 600));
    }

    // --- No selection tests ---

    #[test]
    fn no_selection_shows_prompt() {
        let mut harness = test_harness(vec![]);
        harness.state_mut().selected_target = None;
        harness.run();
        assert!(harness
            .query_by_label("Select a target to configure.")
            .is_some());
    }

    // --- Disappearing client tests ---

    #[test]
    fn disappeared_client_clears_selection() {
        let mut app = AdminPanelState::test_new(vec!["client-a".to_string()]);
        app.select_target(Target::RemoteClient("client-a".to_string()));
        assert_eq!(
            app.selected_target,
            Some(Target::RemoteClient("client-a".to_string()))
        );

        // Render with empty client list -- should invalidate.
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.render(ctx, &[]);
        });
        assert_eq!(app.selected_target, None);
    }

    // --- Modal tests ---

    #[test]
    fn modal_sending_shows_stop_waiting() {
        let mut harness = test_harness(vec![]);
        *harness.state_mut().config_send_state.lock().unwrap() = Some(ConfigSendState::Sending {
            label: "test".to_string(),
        });
        // Use step() instead of run() because the sending state triggers request_repaint.
        harness.step();
        assert!(harness.query_by_label("Stop Waiting").is_some());
    }

    #[test]
    fn modal_success_shows_ok() {
        let mut harness = test_harness(vec![]);
        *harness.state_mut().config_send_state.lock().unwrap() = Some(ConfigSendState::Success {
            label: "test".to_string(),
            message: "Done.".to_string(),
        });
        harness.step();
        assert!(harness.query_by_label("Ok").is_some());
    }

    #[test]
    fn modal_error_shows_message() {
        let mut harness = test_harness(vec![]);
        *harness.state_mut().config_send_state.lock().unwrap() = Some(ConfigSendState::Error {
            label: "test".to_string(),
            message: "Connection refused".to_string(),
        });
        harness.step();
        assert!(harness.query_by_label("Ok").is_some());
        assert!(harness
            .query_by_label("Failed (test):\nConnection refused")
            .is_some());
    }

    // --- Snapshot tests ---

    #[test]
    fn snapshot_monitor_selected() {
        let mut harness = test_harness(vec!["projector-1".to_string(), "projector-2".to_string()]);
        harness.run();
        harness.snapshot("admin_monitor_selected");
    }

    #[test]
    fn snapshot_remote_client_selected() {
        let clients = vec!["projector-1".to_string(), "projector-2".to_string()];
        let mut harness = test_harness(clients);
        harness
            .state_mut()
            .select_target(Target::RemoteClient("projector-1".to_string()));
        harness.run();
        harness.snapshot("admin_remote_selected");
    }

    #[test]
    fn snapshot_no_selection() {
        let mut harness = test_harness(vec!["projector-1".to_string()]);
        harness.state_mut().selected_target = None;
        harness.run();
        harness.snapshot("admin_no_selection");
    }

    #[test]
    fn snapshot_modal_sending() {
        let mut harness = test_harness(vec![]);
        *harness.state_mut().config_send_state.lock().unwrap() = Some(ConfigSendState::Sending {
            label: "projector-1".to_string(),
        });
        harness.step();
        harness.snapshot("admin_modal_sending");
    }

    #[test]
    fn snapshot_modal_success() {
        let mut harness = test_harness(vec![]);
        *harness.state_mut().config_send_state.lock().unwrap() = Some(ConfigSendState::Success {
            label: "projector-1".to_string(),
            message: "Stopped a running show.\nStarting a new show.".to_string(),
        });
        harness.step();
        harness.snapshot("admin_modal_success");
    }

    #[test]
    fn snapshot_modal_error() {
        let mut harness = test_harness(vec![]);
        *harness.state_mut().config_send_state.lock().unwrap() = Some(ConfigSendState::Error {
            label: "projector-1".to_string(),
            message: "Resource temporarily unavailable".to_string(),
        });
        harness.step();
        harness.snapshot("admin_modal_error");
    }
}
