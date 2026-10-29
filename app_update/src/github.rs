use serde::Deserialize;

use crate::{BadRelease, CheckError, ReleaseFeed};

/// The fields of GitHub's release object the updater reads.
#[derive(Debug, Deserialize)]
pub(crate) struct LatestRelease {
    pub tag_name: String,
    pub assets: Vec<ReleaseAssetJson>,
}

/// The fields of GitHub's release asset object the updater reads.
#[derive(Debug, Deserialize)]
pub(crate) struct ReleaseAssetJson {
    pub name: String,
    pub browser_download_url: String,
    pub uploader: Uploader,
}

/// The account that uploaded a release asset.
#[derive(Debug, Deserialize)]
pub(crate) struct Uploader {
    pub login: String,
}

/// The newest published release of `feed`'s repository, read from the REST API at `api_base`.
pub(crate) fn fetch_latest(
    agent: &ureq::Agent,
    api_base: &str,
    feed: &ReleaseFeed,
) -> Result<LatestRelease, CheckError> {
    let url = format!(
        "{api_base}/repos/{}/{}/releases/latest",
        feed.owner, feed.repo
    );
    let mut response = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()?;
    let status = response.status();
    if !status.is_success() {
        return Err(CheckError::Status {
            status: status.as_u16(),
        });
    }
    response.body_mut().read_json().map_err(|e| match e {
        ureq::Error::Json(e) => BadRelease::Unreadable {
            reason: e.to_string(),
        }
        .into(),
        e => CheckError::Network(e),
    })
}
