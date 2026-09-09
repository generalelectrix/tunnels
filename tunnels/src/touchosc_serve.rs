//! The TouchOSC layout this console offers to devices.

use anyhow::{Context, Result};
use touchosc_sync::LayoutServer;

/// The name the layout is given on the device.
const LAYOUT_NAME: &str = "tunnels";

/// The layout offered to devices, as a `.touchosc` ZIP container.
const TEMPLATE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../controller_templates/tunnels.touchosc"
));

/// Start serving the bundled layout, advertising this machine under the name
/// it is known by elsewhere on the network.
pub fn serve_layout() -> Result<LayoutServer> {
    // The Mk1 app expects the raw layout XML rather than the ZIP container it
    // is distributed in.
    let xml = touchosc_sync::extract_layout_xml(TEMPLATE)
        .context("failed to read the bundled TouchOSC layout")?;
    LayoutServer::start(&zero_configure::bare::machine_hostname(), LAYOUT_NAME, &xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled asset must be a container the sync protocol can serve: a
    /// ZIP holding a TouchOSC layout document.
    #[test]
    fn bundled_layout_yields_xml() {
        let xml =
            touchosc_sync::extract_layout_xml(TEMPLATE).expect("the bundled layout should extract");
        let head = String::from_utf8_lossy(&xml[..xml.len().min(256)]);
        assert!(
            head.contains("<layout"),
            "extracted document is not a TouchOSC layout: {head}"
        );
    }
}
