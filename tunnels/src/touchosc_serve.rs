//! Push the bundled TouchOSC layout to a device over the TouchOSC Mk1 editor's
//! layout sync protocol.

use std::io::Read;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use log::{info, warn};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use tiny_http::{Header, Response, Server};

/// The port the TouchOSC Mk1 editor serves layouts on.
const PORT: u16 = 9658;

/// The DNS-SD service type TouchOSC Mk1 browses for when adding a layout.
const SERVICE_TYPE: &str = "_touchosceditor._tcp.local.";

/// The name the layout is given on the device.
const LAYOUT_NAME: &str = "tunnels";

/// How long to wait for the mDNS daemon to confirm the goodbye packet.
const UNREGISTER_TIMEOUT: Duration = Duration::from_millis(500);

/// The layout offered to devices, as a `.touchosc` ZIP container.
const TEMPLATE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../controller_templates/tunnels.touchosc"
));

/// An HTTP server that emulates the TouchOSC Mk1 editor's layout sync
/// protocol. Advertises itself over mDNS as `_touchosceditor._tcp` and serves
/// the layout in response to any request.
pub struct LayoutServer {
    http_server: Arc<Server>,
    thread: Option<JoinHandle<()>>,
    mdns: ServiceDaemon,
    service_fullname: String,
}

impl LayoutServer {
    /// Bind the sync port, register the mDNS service, and begin serving.
    ///
    /// Returns once the server is reachable; requests are handled on a
    /// background thread.
    pub fn start() -> Result<Self> {
        // The Mk1 app expects the raw layout XML rather than the ZIP
        // container it is distributed in.
        let layout_xml = layout_xml().context("failed to read the bundled TouchOSC layout")?;
        let headers = response_headers().context("failed to build the sync response headers")?;

        let http_server = Arc::new(
            Server::http(format!("0.0.0.0:{PORT}"))
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("failed to start the layout sync HTTP server")?,
        );

        let mdns = ServiceDaemon::new().context("failed to start the mDNS daemon")?;
        let instance_name = zero_configure::bare::machine_hostname();
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &local_hostname(),
            (),
            PORT,
            None,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to describe the mDNS service")?
        // Let the daemon track the host's addresses rather than pinning one
        // interface, so the service stays reachable across network changes.
        .enable_addr_auto();
        let service_fullname = service_info.get_fullname().to_string();
        mdns.register(service_info)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("failed to register the mDNS service")?;

        info!("TouchOSC layout server listening on port {PORT}, advertising as {instance_name}.");

        let server = Arc::clone(&http_server);
        let thread = thread::spawn(move || {
            serve_loop(&server, &layout_xml, &headers);
        });

        Ok(Self {
            http_server,
            thread: Some(thread),
            mdns,
            service_fullname,
        })
    }

    /// Stop serving, withdraw the mDNS advertisement, and release the port.
    pub fn stop(&mut self) {
        self.http_server.unblock();
        // Wait for the goodbye packet to go out before tearing the daemon
        // down, so browsing devices drop the entry instead of timing it out.
        if let Ok(unregistered) = self.mdns.unregister(&self.service_fullname) {
            let _ = unregistered.recv_timeout(UNREGISTER_TIMEOUT);
        }
        let _ = self.mdns.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        info!("TouchOSC layout server stopped.");
    }
}

impl Drop for LayoutServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The headers that mark a sync response as a TouchOSC layout download.
struct ResponseHeaders {
    content_type: Header,
    content_disposition: Header,
}

fn response_headers() -> Result<ResponseHeaders> {
    let header = |name: &str, value: &str| {
        Header::from_bytes(name.as_bytes(), value.as_bytes())
            .map_err(|()| anyhow::anyhow!("invalid header {name}: {value}"))
    };
    Ok(ResponseHeaders {
        content_type: header("Content-Type", "application/touchosc")?,
        content_disposition: header(
            "Content-Disposition",
            &format!("attachment; filename=\"{LAYOUT_NAME}.touchosc\""),
        )?,
    })
}

/// Extract the layout XML from the bundled `.touchosc` container.
fn layout_xml() -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(TEMPLATE))
        .context("the layout is not a valid ZIP archive")?;
    let mut entry = archive
        .by_name("index.xml")
        .context("the layout archive has no index.xml")?;
    let mut xml = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut xml)
        .context("failed to decompress index.xml")?;
    Ok(xml)
}

/// This machine's hostname in the form the mDNS daemon expects.
fn local_hostname() -> String {
    let raw = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let short = raw.split('.').next().unwrap_or(&raw);
    format!("{short}.local.")
}

fn serve_loop(server: &Server, layout_xml: &[u8], headers: &ResponseHeaders) {
    for request in server.incoming_requests() {
        info!(
            "TouchOSC layout sync request from {}.",
            request
                .remote_addr()
                .map_or("an unknown address".to_string(), |a| a.to_string()),
        );

        let response = Response::from_data(layout_xml)
            .with_header(headers.content_type.clone())
            .with_header(headers.content_disposition.clone());

        if let Err(e) = request.respond(response) {
            warn!("Failed to respond to a TouchOSC layout sync request: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled asset must be a container the sync protocol can serve: a
    /// ZIP holding a TouchOSC layout document.
    #[test]
    fn bundled_layout_yields_xml() {
        let xml = layout_xml().expect("the bundled layout should extract");
        let head = String::from_utf8_lossy(&xml[..xml.len().min(256)]);
        assert!(
            head.contains("<layout"),
            "extracted document is not a TouchOSC layout: {head}"
        );
    }
}
