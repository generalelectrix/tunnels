//! A window offering a newer release of the app, which installs it and relaunches.

use std::sync::{Arc, Mutex};

use app_update::{AvailableUpdate, DownloadProgress, ReleaseVersion, Updater};
use eframe::egui;
use tunnels_lib::notified::{Notified, NotifiedAtomic};
use tunnels_lib::repaint::RepaintSignal;

/// What the update window ended with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateChoice {
    /// Carry on with the running version.
    Continue,
    /// Exit: the updated app has been launched, or the window was closed.
    Quit,
}

/// Show a window titled `title` offering `update`, installing it if asked. `setup` styles the
/// window.
pub fn run(
    title: &str,
    updater: Updater,
    update: AvailableUpdate,
    setup: impl FnOnce(&egui::Context) + 'static,
) -> eframe::Result<UpdateChoice> {
    let choice = Arc::new(Mutex::new(UpdateChoice::Quit));
    let app_choice = choice.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 160.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            setup(&cc.egui_ctx);
            let repaint: RepaintSignal = {
                let ctx = cc.egui_ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
            Ok(Box::new(UpdateApp {
                updater: Arc::new(updater),
                version: update.version(),
                repaint,
                relaunch: app_update::relaunch,
                choice: app_choice,
                stage: Stage::Offer(update),
            }))
        }),
    )?;
    let choice = *choice
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(choice)
}

struct UpdateApp {
    updater: Arc<Updater>,
    /// The offered release's version.
    version: ReleaseVersion,
    repaint: RepaintSignal,
    /// Starts the updated app.
    relaunch: fn() -> std::io::Result<()>,
    choice: Arc<Mutex<UpdateChoice>>,
    stage: Stage,
}

enum Stage {
    Offer(AvailableUpdate),
    Installing {
        /// Download progress in thousandths.
        permille: Arc<NotifiedAtomic<u32>>,
        /// The install's result, once it has finished.
        outcome: Arc<Notified<Option<Result<(), String>>>>,
    },
    Failed {
        message: String,
    },
    /// The update is installed, but the updated app did not start.
    Installed {
        message: String,
    },
    /// The window is closing.
    Done,
}

impl eframe::App for UpdateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

impl UpdateApp {
    fn ui(&mut self, ctx: &egui::Context) {
        let mut next = None;
        egui::CentralPanel::default().show(ctx, |ui| match &self.stage {
            Stage::Offer(update) => {
                ui.heading(format!("Version {} is available", self.version));
                ui.label(format!("You have {}.", self.updater.current()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Install and relaunch").clicked() {
                        next = Some(self.begin_install(update.clone()));
                    }
                    if ui.button("Not now").clicked() {
                        next = Some(self.finish(ctx, UpdateChoice::Continue));
                    }
                });
            }
            Stage::Installing { permille, outcome } => match &**outcome.load() {
                Some(Ok(())) => {
                    next = Some(match (self.relaunch)() {
                        Ok(()) => self.finish(ctx, UpdateChoice::Quit),
                        Err(e) => Stage::Installed {
                            message: format!(
                                "Updated to {}, but couldn't start it ({e}). Reopen the app.",
                                self.version,
                            ),
                        },
                    });
                }
                Some(Err(message)) => {
                    next = Some(Stage::Failed {
                        message: message.clone(),
                    });
                }
                None => {
                    ui.heading(format!("Installing {}", self.version));
                    ui.add_space(8.0);
                    let fraction = permille.load() as f32 / 1000.0;
                    ui.add(egui::ProgressBar::new(fraction).show_percentage());
                }
            },
            Stage::Failed { message } => {
                ui.heading("Update failed");
                ui.label(message);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Open release page").clicked() {
                        let url = self.updater.release_page_url();
                        if let Err(e) = app_update::open_url(&url) {
                            log::warn!("Couldn't open {url}: {e}");
                        }
                    }
                    if ui.button("Continue").clicked() {
                        next = Some(self.finish(ctx, UpdateChoice::Continue));
                    }
                });
            }
            Stage::Installed { message } => {
                ui.heading("Update installed");
                ui.label(message);
                ui.add_space(8.0);
                if ui.button("Quit").clicked() {
                    next = Some(self.finish(ctx, UpdateChoice::Quit));
                }
            }
            Stage::Done => {}
        });
        if let Some(next) = next {
            self.stage = next;
        }
    }

    /// Start installing `update` on a background thread.
    fn begin_install(&self, update: AvailableUpdate) -> Stage {
        let permille = Arc::new(NotifiedAtomic::new(0, self.repaint.clone()));
        let outcome = Arc::new(Notified::new(None, self.repaint.clone()));
        let job = {
            let updater = self.updater.clone();
            let permille = permille.clone();
            let outcome = outcome.clone();
            move || {
                let on_progress = move |progress: DownloadProgress| {
                    permille.store_if_changed(download_permille(progress));
                };
                let result = updater.install(&update, on_progress);
                outcome.store(Some(result.map_err(|e| e.to_string())));
            }
        };
        match std::thread::Builder::new()
            .name("update-install".to_string())
            .spawn(job)
        {
            Ok(_) => Stage::Installing { permille, outcome },
            Err(e) => Stage::Failed {
                message: format!("Couldn't start the install: {e}"),
            },
        }
    }

    /// Record `choice` and close the window.
    fn finish(&self, ctx: &egui::Context, choice: UpdateChoice) -> Stage {
        *self
            .choice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = choice;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        Stage::Done
    }
}

/// `progress` in thousandths of the archive, or zero when its size is unknown.
fn download_permille(progress: DownloadProgress) -> u32 {
    match progress.total {
        Some(total) if total > 0 => (progress.received.min(total) * 1000 / total) as u32,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FEED: app_update::ReleaseFeed = app_update::ReleaseFeed {
        owner: "owner",
        repo: "repo",
        archive_asset: "Fake.app.tar.gz",
        bundle_name: "Fake.app",
    };

    static RELAUNCHES: AtomicUsize = AtomicUsize::new(0);

    fn counting_relaunch() -> std::io::Result<()> {
        RELAUNCHES.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// A finished install launches the updated app once and closes the window to quit.
    #[test]
    fn a_finished_install_relaunches_once_and_quits() {
        let repaint = tunnels_lib::repaint::noop_repaint();
        let choice = Arc::new(Mutex::new(UpdateChoice::Continue));
        let mut app = UpdateApp {
            updater: Arc::new(Updater::new(FEED, "2026.09.09-1".parse().unwrap())),
            version: "2026.10.01-1".parse().unwrap(),
            repaint: repaint.clone(),
            relaunch: counting_relaunch,
            choice: choice.clone(),
            stage: Stage::Installing {
                permille: Arc::new(NotifiedAtomic::new(1000, repaint.clone())),
                outcome: Arc::new(Notified::new(Some(Ok(())), repaint)),
            },
        };

        let ctx = egui::Context::default();
        let first = ctx.run(egui::RawInput::default(), |ctx| app.ui(ctx));
        let _second = ctx.run(egui::RawInput::default(), |ctx| app.ui(ctx));

        assert_eq!(RELAUNCHES.load(Ordering::Relaxed), 1);
        assert_eq!(*choice.lock().unwrap(), UpdateChoice::Quit);
        assert!(
            first
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|viewport| viewport.commands.contains(&egui::ViewportCommand::Close))
        );
    }

    #[test]
    fn download_permille_is_bounded_and_tolerates_unknown_totals() {
        let at = |received, total| download_permille(DownloadProgress { received, total });
        assert_eq!(at(0, Some(200)), 0);
        assert_eq!(at(50, Some(200)), 250);
        assert_eq!(at(200, Some(200)), 1000);
        assert_eq!(at(300, Some(200)), 1000);
        assert_eq!(at(50, Some(0)), 0);
        assert_eq!(at(50, None), 0);
    }
}
