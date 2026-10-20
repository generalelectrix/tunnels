use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use self_update::http_client::{HeaderMap, HttpClient};
use sha2::Digest;
use tiny_http::{Header, Response, ResponseBox, Server, StatusCode};

use super::*;
use crate::github::{LatestRelease, ReleaseAssetJson};
use crate::transport::CancellableClient;

const FEED: ReleaseFeed = ReleaseFeed {
    owner: "owner",
    repo: "repo",
    archive_asset: "Fake.app.tar.gz",
    bundle_name: "Fake.app",
};

fn v(s: &str) -> ReleaseVersion {
    s.parse().unwrap()
}

/// A local HTTP server answering each request, on its own thread, with `respond(url)`, and
/// recording the `User-Agent` each request carried.
struct TestServer {
    base: String,
    user_agents: Arc<Mutex<Vec<String>>>,
}

fn serve(respond: impl Fn(&str) -> ResponseBox + Send + Sync + 'static) -> TestServer {
    let server = Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr().to_ip().unwrap());
    let user_agents = Arc::new(Mutex::new(Vec::new()));
    let recorded = user_agents.clone();
    let respond = Arc::new(respond);
    thread::spawn(move || {
        for request in server.incoming_requests() {
            if let Some(ua) = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("User-Agent"))
            {
                recorded.lock().unwrap().push(ua.value.to_string());
            }
            let respond = respond.clone();
            thread::spawn(move || {
                let response = respond(request.url());
                let _ = request.respond(response);
            });
        }
    });
    TestServer { base, user_agents }
}

/// Large enough that the server flushes each chunk as it is produced rather than buffering
/// the response.
const CHUNK: usize = 32 * 1024;

/// A body of `chunks` chunks of [`CHUNK`] bytes, pausing `gap` before each chunk after the first.
struct Drip {
    chunks: usize,
    gap: Duration,
    /// Bytes delivered so far.
    sent: usize,
}

impl Read for Drip {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let total = self.chunks * CHUNK;
        if self.sent == total {
            return Ok(0);
        }
        if self.sent > 0 && self.sent.is_multiple_of(CHUNK) {
            thread::sleep(self.gap);
        }
        let rest_of_chunk = CHUNK - self.sent % CHUNK;
        let n = rest_of_chunk.min(buf.len());
        buf[..n].fill(b'x');
        self.sent += n;
        Ok(n)
    }
}

fn drip(chunks: usize, gap: Duration) -> ResponseBox {
    Response::new(
        StatusCode(200),
        vec![],
        Drip {
            chunks,
            gap,
            sent: 0,
        },
        Some(chunks * CHUNK),
        None,
    )
    .boxed()
}

fn read_body(client: &CancellableClient, url: &str) -> io::Result<Vec<u8>> {
    let response = client
        .get(url, &HeaderMap::new(), None)
        .map_err(io::Error::other)?;
    let mut body = Vec::new();
    response.body().read_to_end(&mut body)?;
    Ok(body)
}

#[test]
fn transport_sends_user_agent_maps_status_and_bounds_stalls_not_drips() {
    let server = serve(|url| match url {
        "/drip" => drip(4, Duration::from_millis(300)),
        "/stall" => drip(2, Duration::from_millis(1500)),
        _ => Response::empty(404).boxed(),
    });
    let timeouts = Timeouts {
        stall: Duration::from_millis(700),
        ..Timeouts::default()
    };
    let client = CancellableClient::new(
        transport::client("test-agent/1", timeouts).unwrap(),
        CancelToken::new(),
    );

    // The drip takes ~900 ms in total, longer than the stall limit, but never pauses that long.
    assert_eq!(
        read_body(&client, &format!("{}/drip", server.base))
            .unwrap()
            .len(),
        4 * CHUNK,
    );

    let stalled = read_body(&client, &format!("{}/stall", server.base));
    let timed_out = stalled.as_ref().is_err_and(|e| {
        e.get_ref()
            .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
            .is_some_and(reqwest::Error::is_timeout)
    });
    assert!(timed_out, "a stalled body should time out, got {stalled:?}");

    let missing = client.get(&format!("{}/missing", server.base), &HeaderMap::new(), None);
    assert!(
        matches!(missing, Err(self_update::Error::NotFound { .. })),
        "a 404 should map to NotFound, got {:?}",
        missing.map(|_| ()),
    );

    assert!(
        server
            .user_agents
            .lock()
            .unwrap()
            .iter()
            .all(|ua| ua == "test-agent/1"),
    );
}

#[test]
fn cancelling_fails_the_next_body_read() {
    let server = serve(|_| drip(3, Duration::from_millis(50)));
    let cancel = CancelToken::new();
    let client = CancellableClient::new(
        transport::client("test-agent/1", Timeouts::default()).unwrap(),
        cancel.clone(),
    );
    let response = client.get(&server.base, &HeaderMap::new(), None).unwrap();
    let mut body = response.body();
    let mut buf = [0; 16];
    assert!(body.read(&mut buf).unwrap() > 0);

    cancel.cancel();
    let err = body.read(&mut buf).unwrap_err();
    assert_eq!(err.to_string(), "download cancelled");
}

fn latest(tag: &str, assets: Vec<ReleaseAssetJson>) -> LatestRelease {
    LatestRelease {
        tag_name: tag.to_string(),
        html_url: "https://example.com/release".to_string(),
        assets,
    }
}

fn asset(name: &str, digest: Option<&str>) -> ReleaseAssetJson {
    ReleaseAssetJson {
        name: name.to_string(),
        browser_download_url: format!("https://example.com/{name}"),
        digest: digest.map(str::to_string),
    }
}

#[test]
fn select_offers_only_newer_well_formed_releases() {
    let current = v("2026.09.09-1");
    let good = || {
        vec![
            asset("Other.dmg", Some("sha256:00")),
            asset("Fake.app.tar.gz", Some("sha256:ab")),
        ]
    };

    let update = select(&FEED, current, latest("v2026.10.01-2", good()))
        .unwrap()
        .unwrap();
    assert_eq!(update.version(), v("2026.10.01-2"));
    assert_eq!(update.release.version(), "2026.10.1-2");
    let [offered] = update.release.assets() else {
        panic!("expected exactly the archive asset");
    };
    assert_eq!(offered.name(), "Fake.app.tar.gz");
    assert_eq!(offered.digest(), Some("sha256:ab"));

    for not_newer in ["v2026.09.09-1", "v2026.09.08-3"] {
        assert!(
            select(&FEED, current, latest(not_newer, good()))
                .unwrap()
                .is_none()
        );
    }

    assert_eq!(
        select(&FEED, current, latest("nightly", good())).unwrap_err(),
        BadRelease::Tag {
            tag: "nightly".to_string()
        },
    );
    assert_eq!(
        select(
            &FEED,
            current,
            latest("v2026.10.01-1", vec![asset("Other.dmg", None)])
        )
        .unwrap_err(),
        BadRelease::MissingAsset {
            name: "Fake.app.tar.gz".to_string()
        },
    );
    for digest in [None, Some("md5:ab")] {
        assert_eq!(
            select(
                &FEED,
                current,
                latest("v2026.10.01-1", vec![asset("Fake.app.tar.gz", digest)]),
            )
            .unwrap_err(),
            BadRelease::MissingDigest {
                name: "Fake.app.tar.gz".to_string()
            },
        );
    }
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

/// A server publishing `tag` with `archive` as its bundle asset, advertised with `digest`.
fn publish(tag: &'static str, archive: Vec<u8>, digest: String) -> TestServer {
    let base = Arc::new(Mutex::new(String::new()));
    let base_for_json = base.clone();
    let server = serve(move |url| {
        if url == "/repos/owner/repo/releases/latest" {
            let json = format!(
                r#"{{"tag_name": "{tag}", "html_url": "https://example.com/release",
                    "assets": [{{"name": "Fake.app.tar.gz", "digest": "{digest}",
                                "browser_download_url": "{}/Fake.app.tar.gz"}}]}}"#,
                base_for_json.lock().unwrap(),
            );
            Response::from_string(json)
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                .boxed()
        } else if url == "/Fake.app.tar.gz" {
            Response::from_data(archive.clone()).boxed()
        } else {
            Response::empty(404).boxed()
        }
    });
    *base.lock().unwrap() = server.base.clone();
    server
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", sha2::Sha256::digest(bytes))
}

/// A `Fake.app` in a fresh directory whose executable holds `contents`.
fn installed_bundle(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("Fake.app");
    std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    std::fs::write(bundle.join("Contents/MacOS/fake"), contents).unwrap();
    (dir, bundle)
}

fn installed_contents(bundle: &Path) -> String {
    std::fs::read_to_string(bundle.join("Contents/MacOS/fake")).unwrap()
}

fn updater(server: &TestServer, bundle: &Path) -> Updater {
    Updater {
        api_base: server.base.clone(),
        install_path: Some(bundle.to_path_buf()),
        ..Updater::new(FEED, v("2026.09.09-1"))
    }
}

#[test]
fn check_and_install_swap_in_the_new_bundle() {
    let archive = bundle_archive(b"new");
    let archive_len = archive.len() as u64;
    let server = publish("v2026.10.01-1", archive.clone(), sha256(&archive));
    let (_dir, bundle) = installed_bundle("old");
    let updater = updater(&server, &bundle);

    let update = updater.check().unwrap().unwrap();
    assert_eq!(update.version(), v("2026.10.01-1"));

    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = events.clone();
    updater
        .install(
            &update,
            move |event| recorded.lock().unwrap().push(event),
            &CancelToken::new(),
        )
        .unwrap();

    assert_eq!(installed_contents(&bundle), "new");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(bundle.join("Contents/MacOS/fake"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the executable bit must survive");
    }
    let events = events.lock().unwrap();
    assert_eq!(
        events.iter().rev().take(2).collect::<Vec<_>>(),
        [
            &InstallProgress::Committing,
            &InstallProgress::Downloading {
                received: archive_len,
                total: Some(archive_len)
            },
        ],
    );

    let up_to_date = Updater {
        current: v("2026.10.01-1"),
        ..updater
    };
    assert!(up_to_date.check().unwrap().is_none());
}

#[test]
fn install_leaves_the_bundle_alone_on_bad_digest_or_cancel() {
    let archive = bundle_archive(b"new");
    let server = publish("v2026.10.01-1", archive, sha256(b"something else"));
    let (_dir, bundle) = installed_bundle("old");
    let updater = updater(&server, &bundle);
    let update = updater.check().unwrap().unwrap();

    let mismatch = updater.install(&update, |_| {}, &CancelToken::new());
    assert!(
        matches!(
            mismatch,
            Err(InstallError::Update(
                self_update::Error::ChecksumMismatch { .. }
            ))
        ),
        "got {mismatch:?}",
    );
    assert_eq!(installed_contents(&bundle), "old");

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let result = updater.install(&update, |_| {}, &cancelled);
    assert!(
        matches!(result, Err(InstallError::Cancelled)),
        "got {result:?}"
    );
    assert_eq!(installed_contents(&bundle), "old");
}

#[test]
fn check_reports_http_failures() {
    let server = serve(|_| Response::empty(503).boxed());
    let (_dir, bundle) = installed_bundle("old");
    let result = updater(&server, &bundle).check();
    assert!(
        matches!(result, Err(CheckError::Status { status: 503 })),
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
