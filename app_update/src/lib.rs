//! Self-update for a macOS app bundle published on GitHub Releases.
//!
//! [`Updater::check`] asks GitHub for the repository's latest release and reports it if it is
//! newer than the running build. [`Updater::install`] downloads that release's gzipped tar of the
//! app bundle, checks the bundle's code signature, and swaps it in for the installed bundle,
//! restoring the original if the swap fails. [`relaunch`] starts the installed
//! build.
//!
//! The download goes through this process rather than a browser, so the installed bundle
//! carries no quarantine attribute and Gatekeeper does not hold its launch for approval. An
//! ad-hoc signature suffices.

mod github;
mod version;

#[cfg(test)]
mod tests;

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use version::{ParseVersionError, ReleaseVersion};

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_WEB: &str = "https://github.com";

/// The account the release workflows upload assets as. An asset anyone else uploaded is never
/// installed.
const RELEASE_UPLOADER: &str = "github-actions[bot]";

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
    api_base: String,
    /// Where release assets are downloaded from.
    web_base: String,
    /// Whether every request, including each redirect, must use HTTPS.
    https_only: bool,
    /// The bundle to replace; `None` replaces the bundle containing the running executable.
    install_path: Option<PathBuf>,
    /// Whether a new bundle's code signature must verify before it is installed.
    check_signature: bool,
}

/// A release newer than the running build.
#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    version: ReleaseVersion,
    archive_url: String,
}

impl AvailableUpdate {
    pub fn version(&self) -> ReleaseVersion {
        self.version
    }
}

/// How much of an update's archive has arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    pub received: u64,
    /// The archive's size, when the server reported it.
    pub total: Option<u64>,
}

/// Why [`Updater::check`] could not report on the latest release.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("couldn't reach GitHub: {0}")]
    Network(#[from] ureq::Error),
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
    #[error("GitHub's description of the latest release is unreadable: {reason}")]
    Unreadable { reason: String },
    #[error("release asset {name:?} was uploaded by {login:?}, not the release workflow")]
    UntrustedUploader { name: String, login: String },
    #[error("release asset {name:?} downloads from an unexpected address: {url}")]
    UnexpectedUrl { name: String, url: String },
}

/// Why [`Updater::install`] left the installed bundle as it was.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{} is not inside an app bundle", .0.display())]
    NoBundle(PathBuf),
    #[error("the app is running from a read-only copy; move it to Applications and reopen it")]
    Translocated,
    #[error("couldn't write next to {}: {source}", .bundle.display())]
    NotWritable { bundle: PathBuf, source: io::Error },
    #[error("couldn't download the update: {0}")]
    Download(#[from] ureq::Error),
    #[error("the download answered HTTP {status}")]
    Status { status: u16 },
    #[error("the update archive has no {name}")]
    MissingBundle { name: String },
    #[error("the new app's code signature doesn't verify: {reason}")]
    BadSignature { reason: String },
    #[error(
        "couldn't move the update into place ({error}), and putting the original back failed \
         ({restore}); the original is at {}",
        .original.display()
    )]
    Stranded {
        original: PathBuf,
        error: io::Error,
        restore: io::Error,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Updater {
    /// An updater for `feed` that treats the running build as `current`.
    pub fn new(feed: ReleaseFeed, current: ReleaseVersion) -> Self {
        Self {
            feed,
            current,
            api_base: GITHUB_API.to_string(),
            web_base: GITHUB_WEB.to_string(),
            https_only: true,
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
            "{GITHUB_WEB}/{}/{}/releases/latest",
            self.feed.owner, self.feed.repo
        )
    }

    /// The feed's latest release, if it is newer than the running build. Gives up after
    /// `timeout`.
    pub fn check(&self, timeout: Duration) -> Result<Option<AvailableUpdate>, CheckError> {
        let agent = agent(&self.user_agent(), Some(timeout), self.https_only);
        let latest = github::fetch_latest(&agent, &self.api_base, &self.feed)?;
        let download_prefix = format!(
            "{}/{}/{}/releases/download/",
            self.web_base, self.feed.owner, self.feed.repo
        );
        Ok(select(&self.feed, self.current, latest, &download_prefix)?)
    }

    /// Replace the installed bundle with `update`, reporting the download to `on_progress`. Any
    /// failure leaves the installed bundle as it was.
    pub fn install(
        &self,
        update: &AvailableUpdate,
        on_progress: impl Fn(DownloadProgress),
    ) -> Result<(), InstallError> {
        let installed = self.installed_bundle()?;
        let parent = installed
            .parent()
            .ok_or_else(|| InstallError::NoBundle(installed.clone()))?;
        // Everything is staged beside the installed bundle, so the swap's renames stay on one
        // filesystem. Dropping the directory removes the archive and the replaced bundle.
        let staging = tempfile::Builder::new()
            .prefix(".update-")
            .tempdir_in(parent)
            .map_err(|source| InstallError::NotWritable {
                bundle: installed.clone(),
                source,
            })?;

        let archive = staging.path().join("archive.tar.gz");
        self.download(update, &archive, on_progress)?;
        let mut unpacker = tar::Archive::new(flate2::read::GzDecoder::new(File::open(&archive)?));
        // Nothing unpacked is writable by group or others, whatever the archive says.
        unpacker.set_mask(0o022);
        unpacker.unpack(staging.path())?;
        let new_bundle = staging.path().join(self.feed.bundle_name);
        // A real directory, not a symlink to one somewhere else.
        if !fs::symlink_metadata(&new_bundle).is_ok_and(|meta| meta.is_dir()) {
            return Err(InstallError::MissingBundle {
                name: self.feed.bundle_name.to_string(),
            });
        }
        if self.check_signature {
            verify_signature(&new_bundle)?;
        }
        commit(staging, &installed, &new_bundle, |from, to| {
            fs::rename(from, to)
        })
    }

    /// The bundle an install replaces.
    fn installed_bundle(&self) -> Result<PathBuf, InstallError> {
        if let Some(path) = &self.install_path {
            return Ok(path.clone());
        }
        let exe = std::env::current_exe()?;
        let bundle = containing_bundle(&exe).ok_or_else(|| InstallError::NoBundle(exe.clone()))?;
        // macOS runs a quarantined app from a read-only "App Translocation" mount.
        if bundle
            .components()
            .any(|c| c.as_os_str() == "AppTranslocation")
        {
            return Err(InstallError::Translocated);
        }
        Ok(bundle.to_path_buf())
    }

    /// Download `update`'s archive to `path`.
    fn download(
        &self,
        update: &AvailableUpdate,
        path: &Path,
        on_progress: impl Fn(DownloadProgress),
    ) -> Result<(), InstallError> {
        let mut response = agent(&self.user_agent(), None, self.https_only)
            .get(&update.archive_url)
            .call()?;
        let status = response.status();
        if !status.is_success() {
            return Err(InstallError::Status {
                status: status.as_u16(),
            });
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|length| length.to_str().ok()?.parse().ok());
        let mut body = response.body_mut().as_reader();
        let mut file = File::create(path)?;
        let mut buf = vec![0; 64 * 1024];
        let mut received = 0;
        loop {
            let n = body.read(&mut buf)?;
            let Some(chunk) = buf.get(..n).filter(|chunk| !chunk.is_empty()) else {
                break;
            };
            file.write_all(chunk)?;
            received += n as u64;
            on_progress(DownloadProgress { received, total });
        }
        Ok(())
    }

    fn user_agent(&self) -> String {
        format!("{}-updater/{}", self.feed.repo, self.current)
    }
}

/// An HTTP agent sending `user_agent`, trusting the system's root certificates, giving up on a
/// request after `timeout` when there is one, refusing plain HTTP anywhere along the way if
/// `https_only`, and returning HTTP error statuses as responses.
fn agent(user_agent: &str, timeout: Option<Duration>, https_only: bool) -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    // With native-tls, `PlatformVerifier` means the system's trust store; ureq's default,
    // `WebPki`, panics on first use unless a feature this crate doesn't enable is on.
    let tls = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .user_agent(user_agent)
        .timeout_global(timeout)
        .https_only(https_only)
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

/// `latest` as an update to `current`, or `None` when it is not newer. The update's archive must
/// have been uploaded by [`RELEASE_UPLOADER`] and download from under `download_prefix`.
fn select(
    feed: &ReleaseFeed,
    current: ReleaseVersion,
    latest: github::LatestRelease,
    download_prefix: &str,
) -> Result<Option<AvailableUpdate>, BadRelease> {
    let version: ReleaseVersion = latest.tag_name.parse().map_err(|_| BadRelease::Tag {
        tag: latest.tag_name.clone(),
    })?;
    if version <= current {
        return Ok(None);
    }
    let asset = latest
        .assets
        .into_iter()
        .find(|asset| asset.name == feed.archive_asset)
        .ok_or_else(|| BadRelease::MissingAsset {
            name: feed.archive_asset.to_string(),
        })?;
    if asset.uploader.login != RELEASE_UPLOADER {
        return Err(BadRelease::UntrustedUploader {
            name: asset.name,
            login: asset.uploader.login,
        });
    }
    let url = &asset.browser_download_url;
    let dot_segment = url
        .split('/')
        .any(|segment| segment == "." || segment == "..");
    if !url.starts_with(download_prefix) || dot_segment {
        return Err(BadRelease::UnexpectedUrl {
            name: asset.name,
            url: asset.browser_download_url,
        });
    }
    Ok(Some(AvailableUpdate {
        version,
        archive_url: asset.browser_download_url,
    }))
}

/// Put `new` in `installed`'s place with `rename`, moving the original into `staging` and
/// restoring it if `new` cannot be moved in. If the restore fails too, `staging` is kept so the
/// original survives there.
fn commit(
    staging: tempfile::TempDir,
    installed: &Path,
    new: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), InstallError> {
    let replaced = staging.path().join("replaced");
    rename(installed, &replaced)?;
    let Err(error) = rename(new, installed) else {
        return Ok(());
    };
    match rename(&replaced, installed) {
        Ok(()) => Err(error.into()),
        Err(restore) => Err(InstallError::Stranded {
            original: staging.keep().join("replaced"),
            error,
            restore,
        }),
    }
}

/// Reject a bundle whose code signature does not verify, since macOS refuses to launch one.
#[cfg(target_os = "macos")]
fn verify_signature(bundle: &Path) -> Result<(), InstallError> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(InstallError::BadSignature {
            reason: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

/// Code signatures are only checked on macOS.
#[cfg(not(target_os = "macos"))]
fn verify_signature(_bundle: &Path) -> Result<(), InstallError> {
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
