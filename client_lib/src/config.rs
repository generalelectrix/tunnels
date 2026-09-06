//! Loading and parsing client configurations.
use crate::transform::{Transform, TransformDirection};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::cmp;
use std::fs::File;
use std::io::Read;
use yaml_rust::YamlLoader;

/// The largest valid artnet port address.
///
/// A port address is 15 bits: a net, a sub-net, and a universe.
pub const MAX_ARTNET_PORT_ADDRESS: u16 = 0x7FFF;

/// Settings for a client that doubles as an artnet node.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ArtnetNodeSettings {
    /// The artnet port address whose DMX the node outputs.
    ///
    /// A controller reaches each node at its own address, so nodes only need
    /// to differ here to be told apart by one that broadcasts.
    pub port_address: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Hostname of the machine running the controller.
    pub server_hostname: String,
    /// Virtual video channel to listen to.
    pub video_channel: u64,
    pub x_resolution: u32,
    pub y_resolution: u32,
    /// If true, set the window to fullscreen on creation.
    pub fullscreen: bool,
    /// If true, capture and hide the cursor.
    pub capture_mouse: bool,
    /// Used to rescale unit-scale sizes to the current resolution.
    pub critical_size: f64,
    /// Used to rescale unit-scale lineweights to the current resolution.
    pub thickness_scale: f64,
    /// Computed pixel x-offset of the drawing coordinate system.
    pub x_center: f64,
    /// Computed pixel y-offset of the drawing coordinate system.
    pub y_center: f64,
    /// Geometric transformation to optionally apply to the entire image.
    pub transformation: Option<Transform>,
    /// Serve a DMX interface attached to this machine as an artnet node.
    pub artnet_node: Option<ArtnetNodeSettings>,
    /// Log at debug level?
    pub log_level_debug: bool,
}

impl ClientConfig {
    #[allow(clippy::too_many_arguments)]
    /// Create a configuration from minimal data.
    pub fn new(
        video_channel: u64,
        host: String,
        resolution: Resolution,
        fullscreen: bool,
        capture_mouse: bool,
        transformation: Option<Transform>,
        artnet_node: Option<ArtnetNodeSettings>,
        log_level_debug: bool,
    ) -> ClientConfig {
        let (x_resolution, y_resolution) = resolution;

        ClientConfig {
            server_hostname: host,
            video_channel,
            x_resolution,
            y_resolution,
            fullscreen,
            capture_mouse,
            critical_size: f64::from(cmp::min(x_resolution, y_resolution)),
            thickness_scale: 0.5,
            x_center: f64::from(x_resolution / 2),
            y_center: f64::from(y_resolution / 2),
            transformation,
            artnet_node,
            log_level_debug,
        }
    }

    /// Loads, parses, and returns a config from path.
    /// This method panics if anything is wrong and is only appropriate for use during one-time
    /// initialization.
    pub fn load(video_channel: u64, config_path: &str) -> Result<ClientConfig> {
        let mut config_file = File::open(config_path)?;
        let mut config_file_string = String::new();
        config_file.read_to_string(&mut config_file_string)?;
        let docs = YamlLoader::load_from_str(&config_file_string)?;
        let cfg = &docs[0];
        let x_resolution = cfg["x_resolution"]
            .as_i64()
            .ok_or(anyhow!("Bad x resolution."))? as u32;
        let y_resolution = cfg["y_resolution"]
            .as_i64()
            .ok_or(anyhow!("Bad y resolution."))? as u32;
        let host = cfg["server_hostname"]
            .as_str()
            .ok_or(anyhow!("Hostname missing."))?
            .trim()
            .to_string();

        let flag = |name: &str, missing: &'static str| -> Result<bool> {
            cfg[name].as_bool().ok_or(anyhow!(missing))
        };

        let transformation = if flag("flip_horizontal", "Bad horizontal flip flag.")? {
            Some(Transform::Flip(TransformDirection::Horizontal))
        } else {
            None
        };

        // Absent artnet keys leave the node off, so a config written before
        // clients could serve one still loads.
        let artnet_node = if cfg["artnet_node"].as_bool().unwrap_or(false) {
            let universe = cfg["artnet_universe"].as_i64().unwrap_or(0);
            let port_address = u16::try_from(universe)
                .ok()
                .filter(|address| *address <= MAX_ARTNET_PORT_ADDRESS)
                .ok_or(anyhow!(
                    "Artnet universe {universe} is outside the range 0 to {MAX_ARTNET_PORT_ADDRESS}."
                ))?;
            Some(ArtnetNodeSettings { port_address })
        } else {
            None
        };

        Ok(ClientConfig::new(
            video_channel,
            host,
            (x_resolution, y_resolution),
            flag("fullscreen", "Bad fullscreen flag.")?,
            flag("capture_mouse", "Bad mouse capture flag.")?,
            transformation,
            artnet_node,
            flag("log_level_debug", "Bad log level flag.")?,
        ))
    }
}

pub type Resolution = (u32, u32);

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;

    /// The keys every client config carries, before any artnet settings.
    const BASE_CONFIG: &str = "\
server_hostname: \"127.0.0.1\"
x_resolution: 960
y_resolution: 600
flip_horizontal: false
fullscreen: false
capture_mouse: false
log_level_debug: false
";

    /// Load a config assembled from the base keys plus `artnet`.
    fn load(name: &str, artnet: &str) -> Result<ClientConfig> {
        let path = std::env::temp_dir().join(format!("client_config_{name}.yaml"));
        let mut file = File::create(&path)?;
        write!(file, "{BASE_CONFIG}{artnet}")?;
        drop(file);
        ClientConfig::load(0, path.to_str().expect("the temp path is not utf-8"))
    }

    #[test]
    fn artnet_settings_survive_the_wire() {
        // A config reaches a client over the launcher's stdin.
        let round_trip = |node: Option<ArtnetNodeSettings>| {
            let cfg = ClientConfig::new(
                0,
                "host".into(),
                (640, 480),
                false,
                false,
                None,
                node,
                false,
            );
            let encoded = postcard::to_allocvec(&cfg).expect("failed to encode");
            postcard::from_bytes::<ClientConfig>(&encoded)
                .expect("failed to decode")
                .artnet_node
        };

        assert!(round_trip(None).is_none());
        assert_eq!(
            round_trip(Some(ArtnetNodeSettings { port_address: 291 }))
                .expect("the node settings were lost")
                .port_address,
            291
        );
    }

    #[test]
    fn reads_artnet_settings_from_a_config_file() {
        // A config written before clients could serve a node still loads.
        assert!(load("absent", "").unwrap().artnet_node.is_none());

        assert!(
            load("off", "artnet_node: false\nartnet_universe: 5\n")
                .unwrap()
                .artnet_node
                .is_none()
        );

        // The universe defaults to zero rather than failing to load.
        assert_eq!(
            load("default_universe", "artnet_node: true\n")
                .unwrap()
                .artnet_node
                .expect("the node was not enabled")
                .port_address,
            0
        );

        assert_eq!(
            load("on", "artnet_node: true\nartnet_universe: 291\n")
                .unwrap()
                .artnet_node
                .expect("the node was not enabled")
                .port_address,
            291
        );
    }

    #[test]
    fn rejects_a_universe_no_artnet_node_could_serve() {
        for universe in [-1, i64::from(MAX_ARTNET_PORT_ADDRESS) + 1, 70_000] {
            let err = load(
                &format!("bad_{universe}"),
                &format!("artnet_node: true\nartnet_universe: {universe}\n"),
            )
            .expect_err("an unservable universe was accepted");
            assert_eq!(
                err.to_string(),
                format!("Artnet universe {universe} is outside the range 0 to 32767.")
            );
        }
    }
}
