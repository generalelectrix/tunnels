//! Serve a TouchOSC layout to a device over the TouchOSC Mk1 editor's layout
//! sync protocol.
//!
//! The Mk1 app adds a layout by browsing for `_touchosceditor._tcp` and
//! fetching from whatever it finds, so a host that advertises that service and
//! answers one HTTP request can stand in for the editor.

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

/// How long to wait for the mDNS daemon to confirm the goodbye packet.
const UNREGISTER_TIMEOUT: Duration = Duration::from_millis(500);

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
    /// Bind the sync port, register the mDNS service, and begin serving `xml`.
    ///
    /// `layout_name` is the name the layout is given once it lands on the
    /// device. `xml` is a raw TouchOSC layout document, not a `.touchosc`
    /// container — [`extract_layout_xml`] unwraps one of those.
    ///
    /// The host appears in the device's list of editors under its system
    /// hostname. On macOS that is a different setting from the Computer Name
    /// and the two can disagree, so the entry a device offers may not match
    /// what the machine calls itself elsewhere.
    ///
    /// Returns once the server is reachable; requests are handled on a
    /// background thread.
    pub fn start(layout_name: &str, xml: &[u8]) -> Result<Self> {
        let layout_xml = xml.to_vec();
        let headers =
            response_headers(layout_name).context("failed to build the sync response headers")?;

        let http_server = Arc::new(
            Server::http(format!("0.0.0.0:{PORT}"))
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("failed to start the layout sync HTTP server")?,
        );

        let mdns = ServiceDaemon::new().context("failed to start the mDNS daemon")?;
        let instance_name = short_hostname();
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &format!("{instance_name}.local."),
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

/// Extract the layout document from a `.touchosc` container.
#[cfg(feature = "zip")]
pub fn extract_layout_xml(container: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(container))
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

/// The headers that mark a sync response as a TouchOSC layout download.
struct ResponseHeaders {
    content_type: Header,
    content_disposition: Header,
}

fn response_headers(layout_name: &str) -> Result<ResponseHeaders> {
    let header = |name: &str, value: &str| {
        Header::from_bytes(name.as_bytes(), value.as_bytes())
            .map_err(|()| anyhow::anyhow!("invalid header {name}: {value}"))
    };
    Ok(ResponseHeaders {
        content_type: header("Content-Type", "application/touchosc")?,
        content_disposition: header(
            "Content-Disposition",
            &format!("attachment; filename=\"{layout_name}.touchosc\""),
        )?,
    })
}

/// This machine's hostname, stripped of any domain.
fn short_hostname() -> String {
    let raw = gethostname::gethostname().to_string_lossy().into_owned();
    raw.split('.').next().unwrap_or(&raw).to_string()
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

#[cfg(all(test, feature = "zip"))]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a `.touchosc`-shaped container holding one named entry.
    fn container(entry_name: &str, content: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file(entry_name, options).unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn extraction_recovers_the_document_and_names_what_went_wrong() {
        let xml = b"<layout version=\"18\"><tabpage/></layout>";
        assert_eq!(
            extract_layout_xml(&container("index.xml", xml)).unwrap(),
            xml
        );

        let err = extract_layout_xml(b"not a zip at all").unwrap_err();
        assert!(
            err.to_string().contains("not a valid ZIP archive"),
            "got: {err}"
        );

        let err = extract_layout_xml(&container("layout.xml", xml)).unwrap_err();
        assert!(err.to_string().contains("no index.xml"), "got: {err}");
    }
}
