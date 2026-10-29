use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tiny_http::{Header, Response, ResponseBox, Server};

use super::*;
use crate::github::{LatestRelease, ReleaseAssetJson, Uploader};

const FEED: ReleaseFeed = ReleaseFeed {
    owner: "owner",
    repo: "repo",
    archive_asset: "Fake.app.tar.gz",
    bundle_name: "Fake.app",
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn v(s: &str) -> ReleaseVersion {
    s.parse().unwrap()
}

/// A local HTTP server answering each request with `respond(url)`. Returns its base URL.
fn serve(respond: impl Fn(&str) -> ResponseBox + Send + 'static) -> String {
    let server = Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    thread::spawn(move || {
        for request in server.incoming_requests() {
            let response = respond(request.url());
            let _ = request.respond(response);
        }
    });
    base
}

fn latest(tag: &str, assets: Vec<ReleaseAssetJson>) -> LatestRelease {
    LatestRelease {
        tag_name: tag.to_string(),
        assets,
    }
}

const DOWNLOADS: &str = "https://example.com/owner/repo/releases/download/";

/// An asset named `name`, uploaded by the release workflow, downloading from under [`DOWNLOADS`].
fn asset(name: &str) -> ReleaseAssetJson {
    ReleaseAssetJson {
        name: name.to_string(),
        browser_download_url: format!("{DOWNLOADS}v2026.10.01-2/{name}"),
        uploader: Uploader {
            login: RELEASE_UPLOADER.to_string(),
        },
    }
}

#[test]
fn select_offers_only_newer_well_formed_releases() {
    let current = v("2026.09.09-1");
    let good = || vec![asset("Other.dmg"), asset("Fake.app.tar.gz")];
    let select = |release| select(&FEED, current, release, DOWNLOADS);

    let update = select(latest("v2026.10.01-2", good())).unwrap().unwrap();
    assert_eq!(update.version(), v("2026.10.01-2"));
    assert_eq!(
        update.archive_url,
        format!("{DOWNLOADS}v2026.10.01-2/Fake.app.tar.gz")
    );

    for not_newer in ["v2026.09.09-1", "v2026.09.08-3"] {
        assert!(select(latest(not_newer, good())).unwrap().is_none());
    }

    let mut by_someone_else = asset("Fake.app.tar.gz");
    by_someone_else.uploader.login = "mallory".to_string();
    assert_eq!(
        select(latest("v2026.10.01-1", vec![by_someone_else])).unwrap_err(),
        BadRelease::UntrustedUploader {
            name: "Fake.app.tar.gz".to_string(),
            login: "mallory".to_string(),
        },
    );

    for url in [
        "https://evil.example/owner/repo/releases/download/v1/Fake.app.tar.gz".to_string(),
        format!("{DOWNLOADS}../../../mallory/repo/releases/download/v1/Fake.app.tar.gz"),
        format!("{DOWNLOADS}v1/./Fake.app.tar.gz"),
    ] {
        let mut elsewhere = asset("Fake.app.tar.gz");
        elsewhere.browser_download_url = url.clone();
        assert_eq!(
            select(latest("v2026.10.01-1", vec![elsewhere])).unwrap_err(),
            BadRelease::UnexpectedUrl {
                name: "Fake.app.tar.gz".to_string(),
                url,
            },
        );
    }

    assert_eq!(
        select(latest("nightly", good())).unwrap_err(),
        BadRelease::Tag {
            tag: "nightly".to_string()
        },
    );
    assert_eq!(
        select(latest("v2026.10.01-1", vec![asset("Other.dmg")])).unwrap_err(),
        BadRelease::MissingAsset {
            name: "Fake.app.tar.gz".to_string()
        },
    );
}

/// A gzipped tar of a bundle `Fake.app` whose executable holds `contents`.
fn bundle_archive(contents: &[u8]) -> Vec<u8> {
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    archive
        .append_data(&mut header, "Fake.app/Contents/MacOS/fake", contents)
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap()
}

/// A server publishing `tag` with `archive` as its bundle asset, uploaded by the release
/// workflow.
fn publish(tag: &'static str, archive: Vec<u8>) -> String {
    let base = Arc::new(Mutex::new(String::new()));
    let base_for_json = base.clone();
    let server = serve(move |url| {
        if url == "/repos/owner/repo/releases/latest" {
            let json = format!(
                r#"{{"tag_name": "{tag}",
                    "assets": [{{"name": "Fake.app.tar.gz",
                                "uploader": {{"login": "{RELEASE_UPLOADER}"}},
                                "browser_download_url":
                                    "{}/owner/repo/releases/download/{tag}/Fake.app.tar.gz"}}]}}"#,
                base_for_json.lock().unwrap(),
            );
            Response::from_string(json)
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                .boxed()
        } else if url == format!("/owner/repo/releases/download/{tag}/Fake.app.tar.gz") {
            Response::from_data(archive.clone()).boxed()
        } else {
            Response::empty(404).boxed()
        }
    });
    *base.lock().unwrap() = server.clone();
    server
}

/// A `Fake.app` in a fresh directory whose executable holds `contents`.
fn installed_bundle(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("Fake.app");
    std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    std::fs::write(bundle.join("Contents/MacOS/fake"), contents).unwrap();
    (dir, bundle)
}

/// Assert that no staging directory is left beside `bundle`.
fn assert_no_staging_left(bundle: &Path) {
    let parent = bundle.parent().unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".update-"))
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

fn installed_contents(bundle: &Path) -> String {
    std::fs::read_to_string(bundle.join("Contents/MacOS/fake")).unwrap()
}

/// An updater installing to `bundle` from the plain-HTTP server at `base`, skipping the
/// code-signature check that the unsigned test bundles would fail.
fn updater(base: &str, bundle: &Path) -> Updater {
    Updater {
        api_base: base.to_string(),
        web_base: base.to_string(),
        https_only: false,
        install_path: Some(bundle.to_path_buf()),
        check_signature: false,
        ..Updater::new(FEED, v("2026.09.09-1"))
    }
}

#[test]
fn check_and_install_swap_in_the_new_bundle() {
    let archive = bundle_archive(b"new");
    let archive_len = archive.len() as u64;
    let server = publish("v2026.10.01-1", archive);
    let (_dir, bundle) = installed_bundle("old");
    let updater = updater(&server, &bundle);

    let update = updater.check(TIMEOUT).unwrap().unwrap();
    assert_eq!(update.version(), v("2026.10.01-1"));

    let progress = Arc::new(Mutex::new(Vec::new()));
    let recorded = progress.clone();
    updater
        .install(&update, move |p| recorded.lock().unwrap().push(p))
        .unwrap();

    assert_eq!(installed_contents(&bundle), "new");
    assert_no_staging_left(&bundle);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(bundle.join("Contents/MacOS/fake"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the executable bit must survive");
    }
    assert_eq!(
        progress.lock().unwrap().last(),
        Some(&DownloadProgress {
            received: archive_len,
            total: Some(archive_len)
        }),
    );

    let up_to_date = Updater {
        current: v("2026.10.01-1"),
        ..updater
    };
    assert!(up_to_date.check(TIMEOUT).unwrap().is_none());
}

#[test]
fn install_leaves_the_bundle_alone_on_a_corrupt_archive() {
    let server = publish("v2026.10.01-1", b"not a gzipped tar".to_vec());
    let (_dir, bundle) = installed_bundle("old");
    let updater = updater(&server, &bundle);
    let update = updater.check(TIMEOUT).unwrap().unwrap();

    let result = updater.install(&update, |_| {});
    assert!(matches!(result, Err(InstallError::Io(_))), "got {result:?}",);
    assert_eq!(installed_contents(&bundle), "old");
    assert_no_staging_left(&bundle);
}

#[cfg(target_os = "macos")]
#[test]
fn install_rejects_a_bundle_whose_signature_does_not_verify() {
    let archive = bundle_archive(b"new");
    let server = publish("v2026.10.01-1", archive);
    let (_dir, bundle) = installed_bundle("old");
    let updater = Updater {
        check_signature: true,
        ..updater(&server, &bundle)
    };
    let update = updater.check(TIMEOUT).unwrap().unwrap();

    let result = updater.install(&update, |_| {});
    assert!(
        matches!(result, Err(InstallError::BadSignature { .. })),
        "got {result:?}",
    );
    assert_eq!(installed_contents(&bundle), "old");
}

#[test]
fn check_reports_http_failures_and_unreadable_releases() {
    let (_dir, bundle) = installed_bundle("old");

    let server = serve(|_| Response::empty(503).boxed());
    let result = updater(&server, &bundle).check(TIMEOUT);
    assert!(
        matches!(result, Err(CheckError::Status { status: 503 })),
        "got {result:?}",
    );

    let server = serve(|_| Response::from_string("not json").boxed());
    let result = updater(&server, &bundle).check(TIMEOUT);
    assert!(
        matches!(
            result,
            Err(CheckError::BadRelease(BadRelease::Unreadable { .. }))
        ),
        "got {result:?}",
    );
}

/// A failed swap puts the original bundle back, and if even that fails, the original survives
/// where the error says it is.
#[test]
fn a_failed_swap_keeps_the_original() {
    let (dir, bundle) = installed_bundle("old");
    let missing = dir.path().join("Missing.app");
    let staging = tempfile::tempdir_in(dir.path()).unwrap();
    let result = commit(staging, &bundle, &missing, |from, to| {
        std::fs::rename(from, to)
    });
    assert!(
        matches!(&result, Err(InstallError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound),
        "got {result:?}",
    );
    assert_eq!(installed_contents(&bundle), "old");
    assert_no_staging_left(&bundle);

    // Only the first rename, moving the original aside, succeeds.
    let renames = std::cell::Cell::new(0);
    let staging = tempfile::tempdir_in(dir.path()).unwrap();
    let result = commit(staging, &bundle, &missing, |from, to| {
        renames.set(renames.get() + 1);
        if renames.get() == 1 {
            std::fs::rename(from, to)
        } else {
            Err(std::io::Error::other("injected"))
        }
    });
    let Err(InstallError::Stranded { original, .. }) = result else {
        panic!("expected Stranded, got {result:?}");
    };
    assert_eq!(installed_contents(&original), "old");
}

/// An archive whose bundle is a symlink, here to the installed bundle itself, is refused.
#[test]
fn install_refuses_a_symlinked_bundle() {
    let (_dir, bundle) = installed_bundle("old");
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    archive
        .append_link(&mut header, "Fake.app", &bundle)
        .unwrap();
    let server = publish(
        "v2026.10.01-1",
        archive.into_inner().unwrap().finish().unwrap(),
    );
    let updater = updater(&server, &bundle);
    let update = updater.check(TIMEOUT).unwrap().unwrap();

    let result = updater.install(&update, |_| {});
    assert!(
        matches!(result, Err(InstallError::MissingBundle { .. })),
        "got {result:?}",
    );
    assert_eq!(installed_contents(&bundle), "old");
}

/// ureq's default root certificates panic on first use with native-tls in this crate's feature
/// set, and every other test talks plain HTTP, so pin the agent's trust store here.
#[test]
fn the_agent_trusts_the_system_roots() {
    use ureq::tls::{RootCerts, TlsProvider};
    let agent = agent("test", None, true);
    assert!(agent.config().https_only());
    let tls = agent.config().tls_config();
    assert!(matches!(tls.provider(), TlsProvider::NativeTls));
    assert!(matches!(tls.root_certs(), RootCerts::PlatformVerifier));
}

/// Downloads a real release asset over HTTPS, following GitHub's redirect to its CDN. The latest
/// release's DMG stands in for an update archive.
#[test]
#[ignore = "needs the network"]
fn downloads_a_real_asset_over_https() {
    let feed = ReleaseFeed {
        owner: "generalelectrix",
        repo: "COBRA_COMMANDER",
        archive_asset: "CobraCommander.dmg",
        bundle_name: "Cobra Commander.app",
    };
    let updater = Updater::new(feed, v("2000.01.01-1"));
    let update = updater
        .check(Duration::from_secs(10))
        .unwrap()
        .expect("the latest release is newer than 2000");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("download");

    let progress = Arc::new(Mutex::new(None));
    let recorded = progress.clone();
    updater
        .download(&update, &path, move |p| *recorded.lock().unwrap() = Some(p))
        .unwrap();

    let size = std::fs::metadata(&path).unwrap().len();
    assert_eq!(
        *progress.lock().unwrap(),
        Some(DownloadProgress {
            received: size,
            total: Some(size)
        }),
    );
}

/// Reaches the real GitHub API over HTTPS. The latest release may predate the bundle archive,
/// so only a network or HTTP failure counts as broken.
#[test]
#[ignore = "needs the network"]
fn checks_a_real_feed_over_https() {
    let feed = ReleaseFeed {
        owner: "generalelectrix",
        repo: "COBRA_COMMANDER",
        archive_asset: "CobraCommander.app.tar.gz",
        bundle_name: "Cobra Commander.app",
    };
    let result = Updater::new(feed, v("2000.01.01-1")).check(Duration::from_secs(10));
    assert!(
        matches!(result, Ok(Some(_)) | Err(CheckError::BadRelease(_))),
        "got {result:?}",
    );
}

#[test]
fn finds_the_bundle_enclosing_an_executable() {
    assert_eq!(
        containing_bundle(Path::new(
            "/Applications/Cobra Commander.app/Contents/MacOS/Cobra Commander"
        )),
        Some(Path::new("/Applications/Cobra Commander.app")),
    );
    assert_eq!(containing_bundle(Path::new("/usr/local/bin/tool")), None);
}
