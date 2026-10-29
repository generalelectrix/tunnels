//! An update offer: a row naming the running version and any newer release, and a dialog that
//! installs the release and relaunches.

use std::sync::Arc;

use app_update::{
    AvailableUpdate, CancelToken, CheckError, InstallError, InstallProgress, ReleaseVersion,
    Updater,
};
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
    /// Starts the updated app.
    relaunch: fn() -> std::io::Result<()>,
    /// The newer release, once the check has found one.
    available: Arc<Notified<Option<AvailableUpdate>>>,
    dialog: Dialog,
}

enum Dialog {
    Closed,
    Installing(Install),
    Failed {
        message: String,
    },
    /// The update is installed, but the updated app did not start.
    RelaunchFailed {
        message: String,
    },
    /// The window is closing.
    Exiting,
}

/// One install attempt's progress and result, and what has been asked of it.
struct Install {
    version: ReleaseVersion,
    cancel: CancelToken,
    /// Download progress in thousandths.
    permille: Arc<NotifiedAtomic<u32>>,
    committing: Arc<NotifiedAtomic<bool>>,
    /// How the install ended, once it has.
    outcome: Arc<Notified<Option<Outcome>>>,
    /// A cancel took effect, and the install is winding down.
    cancelling: bool,
    /// The window was asked to close, and closes when the install ends.
    close_requested: bool,
}

impl Install {
    /// Cancel the install if it has not begun committing.
    fn request_cancel(&mut self) {
        if self.cancel.cancel() {
            self.cancelling = true;
        }
    }
}

/// How an install ended.
#[derive(Clone, Debug)]
enum Outcome {
    Installed,
    Cancelled,
    Failed(String),
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
            relaunch: app_update::relaunch,
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

    /// Draw the install dialog if it is open. Returns `true` when the window should close: the
    /// updated app has been launched, or the window was asked to close during an install that
    /// has now ended.
    ///
    /// While an install runs, a request to close the window is held and the install is
    /// cancelled if it still can be. Only requests to close the window itself are held.
    #[must_use]
    pub fn dialog(&mut self, ctx: &egui::Context) -> bool {
        if let Dialog::Installing(install) = &mut self.dialog
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            install.close_requested = true;
            install.request_cancel();
        }

        let next = match &mut self.dialog {
            Dialog::Closed | Dialog::Exiting => None,
            Dialog::Installing(install) => match (**install.outcome.load()).clone() {
                Some(outcome) => Some(ended(outcome, install, self.relaunch, &self.available)),
                None => {
                    installing_ui(ctx, install);
                    None
                }
            },
            Dialog::Failed { message } => failed_ui(ctx, message, &self.updater),
            Dialog::RelaunchFailed { message } => relaunch_failed_ui(ctx, message),
        };
        let Some(next) = next else {
            return false;
        };
        let exit = matches!(next, Dialog::Exiting);
        self.dialog = next;
        exit
    }

    fn begin_install(&mut self, update: AvailableUpdate) {
        let install = Install {
            version: update.version(),
            cancel: CancelToken::new(),
            permille: Arc::new(NotifiedAtomic::new(0, self.repaint.clone())),
            committing: Arc::new(NotifiedAtomic::new(false, self.repaint.clone())),
            outcome: Arc::new(Notified::new(None, self.repaint.clone())),
            cancelling: false,
            close_requested: false,
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
                let ended = match updater.install(&update, on_progress, &cancel) {
                    Ok(()) => Outcome::Installed,
                    Err(InstallError::Cancelled) => Outcome::Cancelled,
                    Err(e) => Outcome::Failed(e.to_string()),
                };
                outcome.store(Some(ended));
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

/// The dialog to show once `install` has ended with `outcome`. A successful install withdraws
/// `available` and starts the updated app with `relaunch`, unless the window is closing.
fn ended(
    outcome: Outcome,
    install: &Install,
    relaunch: fn() -> std::io::Result<()>,
    available: &Notified<Option<AvailableUpdate>>,
) -> Dialog {
    match outcome {
        Outcome::Installed => {
            available.store(None);
            if install.close_requested {
                return Dialog::Exiting;
            }
            match relaunch() {
                Ok(()) => Dialog::Exiting,
                Err(e) => Dialog::RelaunchFailed {
                    message: format!(
                        "Updated to {}, but couldn't start it ({e}). Quit and reopen the app.",
                        install.version,
                    ),
                },
            }
        }
        Outcome::Cancelled | Outcome::Failed(_) if install.close_requested => Dialog::Exiting,
        Outcome::Cancelled => Dialog::Closed,
        Outcome::Failed(message) => Dialog::Failed { message },
    }
}

/// The progress dialog, with a Cancel button until the install commits.
fn installing_ui(ctx: &egui::Context, install: &mut Install) {
    let mut cancel_clicked = false;
    egui::Modal::new(egui::Id::new("update_installing")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading(format!("Installing {}", install.version));
        ui.add_space(8.0);
        if install.cancelling {
            ui.label("Cancelling...");
        } else if install.committing.load() {
            ui.label("Moving the new version into place...");
        } else {
            let fraction = install.permille.load() as f32 / 1000.0;
            ui.add(egui::ProgressBar::new(fraction).show_percentage());
            ui.add_space(4.0);
            ui.label("The app restarts when the update is installed.");
            ui.add_space(8.0);
            cancel_clicked = ui.button("Cancel").clicked();
        }
    });
    if cancel_clicked {
        install.request_cancel();
    }
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

/// The dialog for an installed update that did not start, offering to quit.
fn relaunch_failed_ui(ctx: &egui::Context, message: &str) -> Option<Dialog> {
    let mut quit = false;
    egui::Modal::new(egui::Id::new("update_relaunch_failed")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("Update installed");
        ui.add_space(4.0);
        ui.label(message);
        ui.add_space(8.0);
        quit = ui.button("Quit").clicked();
    });
    quit.then_some(Dialog::Exiting)
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

    fn forbidden_relaunch() -> std::io::Result<()> {
        panic!("the app must not relaunch");
    }

    fn failing_relaunch() -> std::io::Result<()> {
        Err(std::io::Error::other("no LaunchServices"))
    }

    /// A panel running an install, and the handle its worker would report the outcome through.
    fn installing_panel(
        relaunch: fn() -> std::io::Result<()>,
    ) -> (UpdatePanel, Arc<Notified<Option<Outcome>>>) {
        let repaint = tunnels_lib::repaint::noop_repaint();
        let mut panel = UpdatePanel::start(
            Updater::new(FEED, "2026.09.09-1".parse().unwrap()),
            Arc::new(|_, _| {}),
            repaint.clone(),
        );
        panel.relaunch = relaunch;
        let outcome = Arc::new(Notified::new(None, repaint.clone()));
        panel.dialog = Dialog::Installing(Install {
            version: "2026.10.01-1".parse().unwrap(),
            cancel: CancelToken::new(),
            permille: Arc::new(NotifiedAtomic::new(0, repaint.clone())),
            committing: Arc::new(NotifiedAtomic::new(false, repaint)),
            outcome: outcome.clone(),
            cancelling: false,
            close_requested: false,
        });
        (panel, outcome)
    }

    /// What one frame of the panel did: whether it asked the window to close and whether it held
    /// back a close.
    #[derive(Debug, PartialEq)]
    struct Frame {
        exit: bool,
        held_close: bool,
    }

    /// Run one frame of `panel`, with the window asked to close if `close_requested`.
    fn frame(ctx: &egui::Context, panel: &mut UpdatePanel, close_requested: bool) -> Frame {
        let mut input = egui::RawInput::default();
        if close_requested {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let mut exit = false;
        let output = ctx.run(input, |ctx| exit = panel.dialog(ctx));
        let held_close = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|viewport| {
                viewport
                    .commands
                    .contains(&egui::ViewportCommand::CancelClose)
            });
        Frame { exit, held_close }
    }

    const NOTHING: Frame = Frame {
        exit: false,
        held_close: false,
    };
    const EXIT: Frame = Frame {
        exit: true,
        held_close: false,
    };
    const HOLD: Frame = Frame {
        exit: false,
        held_close: true,
    };

    /// Once the updated app is launched, the window's own close must go through, and the app
    /// must be launched only once.
    #[test]
    fn a_finished_install_relaunches_once_and_lets_the_window_close() {
        let ctx = egui::Context::default();
        let (mut panel, outcome) = installing_panel(counting_relaunch);
        outcome.store(Some(Outcome::Installed));

        assert_eq!(frame(&ctx, &mut panel, false), EXIT);
        assert_eq!(frame(&ctx, &mut panel, true), NOTHING);
        assert_eq!(RELAUNCHES.load(Ordering::Relaxed), 1);
    }

    /// Closing the window mid-install cancels the install and closes once the install has
    /// ended, however it ended, without relaunching.
    #[test]
    fn a_close_during_an_install_is_held_until_the_install_ends() {
        let ctx = egui::Context::default();
        for ending in [
            Outcome::Cancelled,
            Outcome::Failed("boom".to_string()),
            Outcome::Installed,
        ] {
            let (mut panel, outcome) = installing_panel(forbidden_relaunch);

            assert_eq!(frame(&ctx, &mut panel, true), HOLD);
            let Dialog::Installing(install) = &panel.dialog else {
                panic!("the install dialog should stay up");
            };
            assert!(install.cancel.is_cancelled());
            assert!(install.cancelling);
            assert_eq!(frame(&ctx, &mut panel, false), NOTHING);

            outcome.store(Some(ending.clone()));
            assert_eq!(frame(&ctx, &mut panel, false), EXIT, "after {ending:?}");
        }
    }

    #[test]
    fn an_install_that_ends_on_its_own_returns_to_the_right_dialog() {
        let ctx = egui::Context::default();

        let (mut panel, outcome) = installing_panel(forbidden_relaunch);
        outcome.store(Some(Outcome::Cancelled));
        assert_eq!(frame(&ctx, &mut panel, false), NOTHING);
        assert!(matches!(panel.dialog, Dialog::Closed));

        let (mut panel, outcome) = installing_panel(forbidden_relaunch);
        outcome.store(Some(Outcome::Failed("boom".to_string())));
        assert_eq!(frame(&ctx, &mut panel, false), NOTHING);
        assert!(matches!(&panel.dialog, Dialog::Failed { message } if message == "boom"));

        let (mut panel, outcome) = installing_panel(failing_relaunch);
        outcome.store(Some(Outcome::Installed));
        assert_eq!(frame(&ctx, &mut panel, false), NOTHING);
        assert!(
            matches!(&panel.dialog, Dialog::RelaunchFailed { message } if message.contains("no LaunchServices"))
        );
    }

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
