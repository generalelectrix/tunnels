use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::blocking::Client;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

/// A shared flag that stops an update while it is still downloading.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the update holding a clone of this token to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// How long the updater's requests may take.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// Longest wait to establish a connection.
    pub connect: Duration,
    /// Longest time for the whole release lookup, response body included.
    pub lookup: Duration,
    /// Longest wait for a response, or for each next piece of its body.
    pub stall: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            lookup: Duration::from_secs(10),
            stall: Duration::from_secs(20),
        }
    }
}

/// A client sending `user_agent` and enforcing `timeouts.connect` and `timeouts.stall`.
///
/// The blocking client applies its timeout separately to the wait for a response and to each
/// body read, so `stall` bounds a hung transfer without capping a slow one.
pub(crate) fn client(user_agent: &str, timeouts: Timeouts) -> reqwest::Result<Client> {
    Client::builder()
        .user_agent(user_agent)
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.stall)
        .build()
}

/// An [`HttpClient`] whose response bodies stop reading once a [`CancelToken`] is cancelled.
pub(crate) struct CancellableClient {
    client: Client,
    cancel: CancelToken,
}

impl CancellableClient {
    pub(crate) fn new(client: Client, cancel: CancelToken) -> Self {
        Self { client, cancel }
    }
}

impl HttpClient for CancellableClient {
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        timeout: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        let mut request = self.client.get(url).headers(headers.clone());
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().map_err(self_update::Error::transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(self_update::Error::http_status_error_with_headers(
                status.as_u16(),
                url,
                response.headers(),
            ));
        }
        Ok(Box::new(CancellableResponse {
            headers: response.headers().clone(),
            body: CancellableBody {
                response,
                cancel: self.cancel.clone(),
            },
        }))
    }
}

struct CancellableResponse {
    headers: HeaderMap,
    body: CancellableBody,
}

impl HttpResponse for CancellableResponse {
    fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    fn body(self: Box<Self>) -> Box<dyn Read> {
        Box::new(self.body)
    }
}

/// A response body that fails its next read once its token is cancelled.
struct CancellableBody {
    response: reqwest::blocking::Response,
    cancel: CancelToken,
}

impl Read for CancellableBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            // Not `Interrupted`: `Read` callers retry that kind.
            return Err(io::Error::other("download cancelled"));
        }
        self.response.read(buf)
    }
}
