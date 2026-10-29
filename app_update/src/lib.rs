//! Self-update for a macOS app bundle published on GitHub Releases.
//!
//! [`Updater::check`] asks GitHub for the repository's latest release and reports it if it is
//! newer than the running build. [`Updater::install`] downloads that release's gzipped tar of the
//! app bundle, verifies it against the digest GitHub publishes, and swaps it in for the
//! installed bundle as one unit, restoring the original if any step fails. [`relaunch`] starts
//! the installed build.
//!
//! Releases are versioned `YYYY.MM.DD-N` ([`ReleaseVersion`]). The download and swap are
//! [`self_update`]'s, which compares versions as semver, so it is handed each version's
//! [`ReleaseVersion::semver_key`].
//!
//! The download goes through this process rather than a browser, so the installed bundle
//! carries no quarantine attribute and Gatekeeper does not hold its launch for approval. An
//! ad-hoc signature suffices.

mod github;
mod transport;
mod version;

#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use self_update::{Release, ReleaseAsset, ReleaseSource};

pub use transport::{CancelToken, Timeouts};
pub use version::{ParseVersionError, ReleaseVersion};

const GITHUB_API: &str = "https://api.github.com";

/// A GitHub repository whose releases each carry a gzipped tar of an app bundle.
#[derive(Clone, Copy, Debug)]
pub struct ReleaseFeed {
    /// The account owning the repository, e.g. `generalelectrix`.
    pub owner: &'static str,
    /// The repository name, e.g. `COBRA_COMMANDER`.
    pub repo: &'static str,
    /// The name of the release asset holding the archive, e.g. `CobraCommander.app.tar.gz`.
    pub archive_asset: &'static str,
    /// The bundle directory at the archive's root, e.g. `Cobra Commander.app`.
    pub bundle_name: &'static str,
}

/// Checks a [`ReleaseFeed`] for releases newer than the running build and installs them.
#[derive(Clone, Debug)]
pub struct Updater {
    feed: ReleaseFeed,
    current: ReleaseVersion,
    timeouts: Timeouts,
    api_base: String,
    /// The bundle to replace; `None` replaces the bundle containing the running executable.
    install_path: Option<PathBuf>,
    /// Whether a new bundle's code signature must verify before it is installed.
    check_signature: bool,
}

/// A release newer than the running build.
#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    version: ReleaseVersion,
    release: Release,
}

impl AvailableUpdate {
    pub fn version(&self) -> ReleaseVersion {
        self.version
    }
}

/// How far an [`Updater::install`] has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallProgress {
    /// `received` bytes of the archive have arrived, of `total` when the server said.
    Downloading { received: u64, total: Option<u64> },
    /// The downloaded bundle has passed verification and is moving into place, past the point a
    /// cancel can stop.
    Committing,
}

/// Why [`Updater::check`] could not report on the latest release.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("couldn't reach GitHub: {0}")]
    Network(#[from] reqwest::Error),
    #[error("GitHub answered HTTP {status}")]
    Status { status: u16 },
    #[error(transparent)]
    BadRelease(#[from] BadRelease),
}

/// A latest release that does not have the shape the updater installs from.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BadRelease {
    #[error("the latest release's tag {tag:?} is not a YYYY.MM.DD-N version")]
    Tag { tag: String },
    #[error("the latest release has no asset named {name:?}")]
    MissingAsset { name: String },
    #[error("release asset {name:?} has no sha256 digest")]
    MissingDigest { name: String },
    #[error("GitHub's description of the latest release is unreadable: {reason}")]
    Unreadable { reason: String },
}

/// Why [`Updater::install`] left the installed bundle as it was.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the update was cancelled")]
    Cancelled,
    #[error("couldn't set up the download: {0}")]
    Client(#[from] reqwest::Error),
    #[error("the installed version is already {0} or newer")]
    NotNewer(ReleaseVersion),
    #[error(transparent)]
    Update(#[from] self_update::Error),
}

impl Updater {
    /// An updater for `feed` that treats the running build as `current`.
    pub fn new(feed: ReleaseFeed, current: ReleaseVersion) -> Self {
        Self {
            feed,
            current,
            timeouts: Timeouts::default(),
            api_base: GITHUB_API.to_string(),
            install_path: None,
            check_signature: true,
        }
    }

    pub fn current(&self) -> ReleaseVersion {
        self.current
    }

    /// The page listing the latest release, for installing by hand.
    pub fn release_page_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/latest",
            self.feed.owner, self.feed.repo
        )
    }

    /// The feed's latest release, if it is newer than the running build.
    pub fn check(&self) -> Result<Option<AvailableUpdate>, CheckError> {
        let client = transport::client(&self.user_agent(), self.timeouts)?;
        let latest =
            github::fetch_latest(&client, &self.api_base, &self.feed, self.timeouts.lookup)?;
        Ok(select(&self.feed, self.current, latest)?)
    }

    /// Replace the installed bundle with `update`, reporting progress to `on_progress`.
    ///
    /// `cancel` can stop the install until it reports [`InstallProgress::Committing`]. Any
    /// failure leaves the installed bundle as it was.
    pub fn install(
        &self,
        update: &AvailableUpdate,
        on_progress: impl Fn(InstallProgress) + Send + Sync + 'static,
        cancel: &CancelToken,
    ) -> Result<(), InstallError> {
        let client = transport::client(&self.user_agent(), self.timeouts)?;
        let on_progress = Arc::new(on_progress);

        let mut builder = self_update::backends::custom::Update::configure();
        builder
            // Install exactly the release that was offered, without asking GitHub again.
            .source(Prefetched(update.release.clone()))
            .bin_name(self.feed.bundle_name)
            .current_version(self.current.semver_key())
            .bundle_path_in_archive(self.feed.bundle_name)
            .http_client(Arc::new(transport::CancellableClient::new(
                client,
                cancel.clone(),
            )))
            // The offered release carries only the bundle archive.
            .asset_matcher(|assets| assets.first().cloned())
            .check_install_path_writable(true)
            .retries(0)
            .unattended()
            .progress_callback({
                let on_progress = on_progress.clone();
                move |received, total| on_progress(InstallProgress::Downloading { received, total })
            })
            .verify_binary({
                let cancel = cancel.clone();
                let check_signature = self.check_signature;
                move |staged_bundle| {
                    if check_signature {
                        verify_signature(staged_bundle)?;
                    }
                    if !cancel.begin_commit() {
                        return Err(self_update::Error::verification_rejected("cancelled"));
                    }
                    on_progress(InstallProgress::Committing);
                    Ok(())
                }
            });
        if let Some(install_path) = &self.install_path {
            builder.bundle_install_path(install_path);
        }

        match builder.build().and_then(|update| update.update_extended()) {
            Ok(status) => status
                .into_updated_release()
                .map(|_| ())
                .ok_or(InstallError::NotNewer(update.version)),
            Err(_) if cancel.is_cancelled() => Err(InstallError::Cancelled),
            Err(e) => Err(e.into()),
        }
    }

    fn user_agent(&self) -> String {
        format!("{}-updater/{}", self.feed.repo, self.current)
    }
}

/// `latest` as an update to `current`, or `None` when it is not newer.
fn select(
    feed: &ReleaseFeed,
    current: ReleaseVersion,
    latest: github::LatestRelease,
) -> Result<Option<AvailableUpdate>, BadRelease> {
    let bad_tag = || BadRelease::Tag {
        tag: latest.tag_name.clone(),
    };
    let version: ReleaseVersion = latest.tag_name.parse().map_err(|_| bad_tag())?;
    if version <= current {
        return Ok(None);
    }
    let asset = latest
        .assets
        .iter()
        .find(|asset| asset.name == feed.archive_asset)
        .ok_or_else(|| BadRelease::MissingAsset {
            name: feed.archive_asset.to_string(),
        })?;
    let digest = asset
        .digest
        .as_deref()
        .filter(|digest| digest.starts_with("sha256:"))
        .ok_or_else(|| BadRelease::MissingDigest {
            name: asset.name.clone(),
        })?;
    let release = Release::builder()
        .version(version.semver_key())
        .name(version.to_string())
        .release_notes_url(latest.html_url.clone())
        .asset(ReleaseAsset::new(&asset.name, &asset.browser_download_url).with_digest(digest))
        .build()
        .map_err(|_| bad_tag())?;
    Ok(Some(AvailableUpdate { version, release }))
}

/// A release source serving one fixed release.
struct Prefetched(Release);

impl ReleaseSource for Prefetched {
    fn get_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(vec![self.0.clone()])
    }
}

/// Reject a staged bundle whose code signature does not verify, since macOS refuses to launch
/// one.
#[cfg(target_os = "macos")]
fn verify_signature(bundle: &Path) -> self_update::Result<()> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(self_update::Error::verification_rejected(format!(
            "the new app's code signature doesn't verify: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Code signatures are only checked on macOS.
#[cfg(not(target_os = "macos"))]
fn verify_signature(_bundle: &Path) -> self_update::Result<()> {
    Ok(())
}

/// Start a new instance of the bundle containing the running executable.
pub fn relaunch() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let bundle = containing_bundle(&exe).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not inside an app bundle", exe.display()),
        )
    })?;
    // `-n`: a new instance, since otherwise LaunchServices reactivates this one.
    open(&[Path::new("-n").as_os_str(), bundle.as_os_str()])
}

/// Open `url` in the default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    open(&[std::ffi::OsStr::new(url)])
}

/// The nearest `.app` directory enclosing `path`.
fn containing_bundle(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|ancestor| ancestor.extension().is_some_and(|ext| ext == "app"))
}

/// Hand `args` to LaunchServices via `open`.
#[cfg(target_os = "macos")]
fn open(args: &[&std::ffi::OsStr]) -> io::Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .args(args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("open exited with {status}")))
    }
}

/// LaunchServices is macOS-only.
#[cfg(not(target_os = "macos"))]
fn open(_args: &[&std::ffi::OsStr]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "opening via LaunchServices requires macOS",
    ))
}
