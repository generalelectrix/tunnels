//! egui-based panel for administering tunnel clients.

use crate::bootstrap_controller::BootstrapController;
use crate::ui_util::GuiContext;
use client_lib::config::ClientConfig;
use client_lib::transform::{Transform, TransformDirection};
use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Abstraction over the network admin operations so we can mock them in tests.
pub trait AdminService: Send + Sync {
    fn clients(&self) -> Vec<String>;
    fn push_config(
        &self,
        name: &str,
        binary_path: &Path,
        config: ClientConfig,
    ) -> anyhow::Result<String>;
}

impl AdminService for BootstrapController {
    fn clients(&self) -> Vec<String> {
        self.list()
    }
    fn push_config(
        &self,
        name: &str,
        binary_path: &Path,
        config: ClientConfig,
    ) -> anyhow::Result<String> {
        let stdin_payload = rmp_serde::to_vec(&config)?;
        self.push_binary(name, binary_path, &["monitor"], &stdin_payload)
    }
}

/// The selected target for configuration: either a local monitor or a remote client.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Monitor,
    RemoteClient(String),
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

/// Find the tunnelclient binary as a sibling of the current executable.
fn tunnelclient_path() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Failed to determine current executable path: {e}"))?;
    let dir = exe.parent().unwrap_or(std::path::Path::new("."));
    let sibling = dir.join("tunnelclient");
    if sibling.exists() {
        Ok(sibling)
    } else {
        Err(format!("tunnelclient not found at {}", sibling.display()))
    }
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

    // Local monitor child processes, killed on drop.
    monitor_children: Arc<Mutex<Vec<Child>>>,
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
            monitor_children: Arc::new(Mutex::new(Vec::new())),
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
    fn perform_action<App>(&mut self, gui: &mut GuiContext<'_, App>)
    where
        App: gui_common::background_task::Project<Self> + gui_common::UserNotify + 'static,
    {
        match &self.selected_target {
            Some(Target::Monitor) => self.launch_monitor(gui),
            Some(Target::RemoteClient(_)) => self.send_config(gui),
            None => {}
        }
    }

    /// Launch a local monitor subprocess.
    fn launch_monitor<App>(&mut self, gui: &mut GuiContext<'_, App>)
    where
        App: gui_common::background_task::Project<Self> + gui_common::UserNotify + 'static,
    {
        let config = match self.build_config() {
            Ok(cfg) => cfg,
            Err(e) => {
                gui.report_error(e);
                return;
            }
        };

        let children = self.monitor_children.clone();
        gui.dispatch_notify("Launching monitor", move || {
            let serialized = rmp_serde::to_vec(&config)
                .map_err(|e| anyhow::anyhow!("Failed to serialize config: {e}"))?;

            let exe = tunnelclient_path().map_err(|e| anyhow::anyhow!(e))?;

            let mut child = Command::new(exe)
                .arg("monitor")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to spawn monitor: {e}"))?;

            {
                let mut stdin = child.stdin.take().unwrap();
                stdin
                    .write_all(&serialized)
                    .map_err(|e| anyhow::anyhow!("Failed to write config to monitor: {e}"))?;
            }

            let stdout = child.stdout.take().unwrap();
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .map_err(|e| anyhow::anyhow!("Failed to read monitor status: {e}"))?;

            let line = line.trim();
            if line == "OK" {
                children.lock().unwrap().push(child);
                Ok("Monitor launched successfully.".to_string())
            } else if let Some(err) = line.strip_prefix("ERROR: ") {
                anyhow::bail!(err.to_string())
            } else if line.is_empty() {
                anyhow::bail!("Monitor process exited without status.")
            } else {
                anyhow::bail!("Unexpected monitor response: {line}")
            }
        });
    }

    /// Send configuration to the selected remote client on a background thread.
    fn send_config<App>(&mut self, gui: &mut GuiContext<'_, App>)
    where
        App: gui_common::background_task::Project<Self> + gui_common::UserNotify + 'static,
    {
        let client_name = match &self.selected_target {
            Some(Target::RemoteClient(name)) => name.clone(),
            _ => return,
        };

        let config = match self.build_config() {
            Ok(cfg) => cfg,
            Err(e) => {
                gui.report_error(e);
                return;
            }
        };

        let admin = self.admin_service.clone();
        gui.dispatch_notify(format!("Sending config to {client_name}"), move || {
            let exe = tunnelclient_path().map_err(|e| anyhow::anyhow!(e))?;
            admin.push_config(&client_name, &exe, config)
        });
    }

    /// Render the full admin UI. Called by `ConfigApp::update` and by test harnesses.
    pub(crate) fn render<App>(
        &mut self,
        ctx: &egui::Context,
        clients: &[String],
        gui: &mut GuiContext<'_, App>,
    ) where
        App: gui_common::background_task::Project<Self> + gui_common::UserNotify + 'static,
    {
        // Invalidate selection if the selected remote client has disappeared.
        if let Some(Target::RemoteClient(ref name)) = self.selected_target
            && !clients.iter().any(|c| c == name)
        {
            self.selected_target = None;
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
                .add_enabled(gui.task.is_none(), egui::Button::new(button_label))
                .clicked()
            {
                self.perform_action(gui);
            }
        });
    }
}

impl Drop for AdminPanelState {
    fn drop(&mut self) {
        let mut children = self.monitor_children.lock().unwrap();
        for child in children.iter_mut() {
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use gui_common::MessageModal;
    use gui_common::background_task::BlockingBackgroundTask;
    use tunnels::control::mock::auto_respond_client;

    struct MockAdminService {
        clients: Vec<String>,
    }

    impl AdminService for MockAdminService {
        fn clients(&self) -> Vec<String> {
            self.clients.clone()
        }
        fn push_config(
            &self,
            _name: &str,
            _binary_path: &Path,
            _config: ClientConfig,
        ) -> anyhow::Result<String> {
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

    /// Dummy app type for tests.
    struct TestApp {
        admin: AdminPanelState,
    }

    gui_common::impl_project!(TestApp, admin: AdminPanelState);

    impl gui_common::UserNotify for TestApp {
        fn notify(&mut self, _title: &str, _message: &str) {}
        fn notify_error(&mut self, _error: anyhow::Error) {}
    }

    fn test_harness(clients: Vec<String>) -> Harness<'static, AdminPanelState> {
        let clients_for_render = clients.clone();
        let client = auto_respond_client();
        let mut modal = MessageModal::default();
        let mut task: Option<BlockingBackgroundTask<TestApp>> = None;
        let harness = Harness::new_ui_state(
            move |ui, app: &mut AdminPanelState| {
                let mut gui = GuiContext {
                    modal: &mut modal,
                    client: &client,
                    task: &mut task,
                };
                app.render::<TestApp>(ui.ctx(), &clients_for_render, &mut gui);
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
        assert!(
            harness
                .query_by_label("Select a target to configure.")
                .is_some()
        );
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
        let client = auto_respond_client();
        let mut modal = MessageModal::default();
        let mut task: Option<BlockingBackgroundTask<TestApp>> = None;
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let mut gui = GuiContext {
                modal: &mut modal,
                client: &client,
                task: &mut task,
            };
            app.render::<TestApp>(ctx, &[], &mut gui);
        });
        assert_eq!(app.selected_target, None);
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
}
