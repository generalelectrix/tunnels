//! An update offer: a row naming the running version and any newer release, and a dialog that
//! installs the release and relaunches.

use std::sync::Arc;

use app_update::{AvailableUpdate, CancelToken, CheckError, InstallProgress, Updater};
use eframe::egui;
use tunnels_lib::notified::{Notified, NotifiedAtomic};
use tunnels_lib::repaint::RepaintSignal;

/// Runs a job on a background thread with the given name.
pub type Spawner = Arc<dyn Fn(&'static str, Box<dyn FnOnce() + Send>) + Send + Sync>;

/// The release check's result and any install in progress for one [`Updater`].
pub struct UpdatePanel {
    updater: Arc<Updater>,
    spawn: Spawner,
    repaint: RepaintSignal,
    /// The newer release, once the check has found one.
    available: Arc<Notified<Option<AvailableUpdate>>>,
    dialog: Dialog,
}

enum Dialog {
    Closed,
    Installing(Install),
    Failed { message: String },
}

/// One install attempt's progress and result.
struct Install {
    version: app_update::ReleaseVersion,
    cancel: CancelToken,
    /// Download progress in thousandths.
    permille: Arc<NotifiedAtomic<u32>>,
    committing: Arc<NotifiedAtomic<bool>>,
    /// The install's result, once it has finished.
    outcome: Arc<Notified<Option<Result<(), String>>>>,
}

impl UpdatePanel {
    /// A panel for `updater`, checking its feed on a thread from `spawn`. `repaint` wakes the
    /// GUI drawing the panel.
    pub fn start(updater: Updater, spawn: Spawner, repaint: RepaintSignal) -> Self {
        let updater = Arc::new(updater);
        let available = Arc::new(Notified::new(None, repaint.clone()));
        let check = {
            let updater = updater.clone();
            let available = available.clone();
            move || match updater.check() {
                Ok(Some(update)) => available.store(Some(update)),
                Ok(None) => {}
                Err(CheckError::BadRelease(e)) => log::warn!("Update check: {e}"),
                Err(e) => log::info!("Update check failed: {e}"),
            }
        };
        spawn("update-check", Box::new(check));
        Self {
            updater,
            spawn,
            repaint,
            available,
            dialog: Dialog::Closed,
        }
    }

    /// The running version, followed by the newer release and an install button when there
    /// is one.
    pub fn row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.updater.current().to_string()).weak());
            let available = self.available.load();
            if let Some(update) = available.as_ref() {
                ui.label(format!("Update {} available", update.version()));
                if ui.button("Install update").clicked() {
                    self.begin_install(update.clone());
                }
            }
        });
    }

    /// Draw the install dialog if it is open. Returns `true` once the updated app has been
    /// launched, when this process should exit.
    #[must_use]
    pub fn dialog(&mut self, ctx: &egui::Context) -> bool {
        let next = match &self.dialog {
            Dialog::Closed => return false,
            Dialog::Installing(install) => match &**install.outcome.load() {
                Some(Ok(())) => match app_update::relaunch() {
                    Ok(()) => return true,
                    Err(e) => Some(Dialog::Failed {
                        message: format!(
                            "Updated to {}, but couldn't relaunch ({e}). Quit and reopen the app.",
                            install.version,
                        ),
                    }),
                },
                Some(Err(message)) => Some(Dialog::Failed {
                    message: message.clone(),
                }),
                None => installing_ui(ctx, install).then_some(Dialog::Closed),
            },
            Dialog::Failed { message } => failed_ui(ctx, message, &self.updater),
        };
        if let Some(next) = next {
            self.dialog = next;
        }
        false
    }

    /// Keep the window open while the install is moving bundles into place, since exiting
    /// then could leave no app installed.
    pub fn hold_close(&self, ctx: &egui::Context) {
        let committing =
            matches!(&self.dialog, Dialog::Installing(install) if install.committing.load());
        if committing && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    fn begin_install(&mut self, update: AvailableUpdate) {
        let install = Install {
            version: update.version(),
            cancel: CancelToken::new(),
            permille: Arc::new(NotifiedAtomic::new(0, self.repaint.clone())),
            committing: Arc::new(NotifiedAtomic::new(false, self.repaint.clone())),
            outcome: Arc::new(Notified::new(None, self.repaint.clone())),
        };
        let job = {
            let updater = self.updater.clone();
            let cancel = install.cancel.clone();
            let permille = install.permille.clone();
            let committing = install.committing.clone();
            let outcome = install.outcome.clone();
            move || {
                let on_progress = move |progress| match progress {
                    InstallProgress::Downloading { received, total } => {
                        permille.store_if_changed(download_permille(received, total));
                    }
                    InstallProgress::Committing => committing.store(true),
                };
                let result = updater.install(&update, on_progress, &cancel);
                outcome.store(Some(result.map_err(|e| e.to_string())));
            }
        };
        (self.spawn)("update-install", Box::new(job));
        self.dialog = Dialog::Installing(install);
    }
}

impl Drop for UpdatePanel {
    /// Stop a download still in progress.
    fn drop(&mut self) {
        if let Dialog::Installing(install) = &self.dialog {
            install.cancel.cancel();
        }
    }
}

/// The progress dialog. Returns `true` when the user cancels.
fn installing_ui(ctx: &egui::Context, install: &Install) -> bool {
    let mut cancelled = false;
    egui::Modal::new(egui::Id::new("update_installing")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading(format!("Installing {}", install.version));
        ui.add_space(8.0);
        if install.committing.load() {
            ui.label("Moving the new version into place...");
        } else {
            let fraction = install.permille.load() as f32 / 1000.0;
            ui.add(egui::ProgressBar::new(fraction).show_percentage());
            ui.add_space(4.0);
            ui.label("The app restarts when the update is installed.");
            ui.add_space(8.0);
            cancelled = ui.button("Cancel").clicked();
        }
    });
    if cancelled {
        install.cancel.cancel();
    }
    cancelled
}

/// The failure dialog, offering the release page for installing by hand. Returns the dialog to
/// show next, if it changes.
fn failed_ui(ctx: &egui::Context, message: &str, updater: &Updater) -> Option<Dialog> {
    let mut next = None;
    egui::Modal::new(egui::Id::new("update_failed")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("Update failed");
        ui.add_space(4.0);
        ui.label(message);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Open release page").clicked() {
                let url = updater.release_page_url();
                next = Some(match app_update::open_url(&url) {
                    Ok(()) => Dialog::Closed,
                    Err(e) => Dialog::Failed {
                        message: format!("Couldn't open {url}: {e}"),
                    },
                });
            }
            if ui.button("Close").clicked() {
                next = Some(Dialog::Closed);
            }
        });
    });
    next
}

/// `received` of `total` bytes in thousandths, or zero when the total is unknown.
fn download_permille(received: u64, total: Option<u64>) -> u32 {
    match total {
        Some(total) if total > 0 => (received.min(total) * 1000 / total) as u32,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_permille_is_bounded_and_tolerates_unknown_totals() {
        assert_eq!(download_permille(0, Some(200)), 0);
        assert_eq!(download_permille(50, Some(200)), 250);
        assert_eq!(download_permille(200, Some(200)), 1000);
        assert_eq!(download_permille(300, Some(200)), 1000);
        assert_eq!(download_permille(50, Some(0)), 0);
        assert_eq!(download_permille(50, None), 0);
    }
}
