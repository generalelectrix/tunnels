//! Loading and parsing client configurations.
use crate::draw::{Transform, TransformDirection};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::cmp;
use std::fs::File;
use std::io::Read;
use std::time::Duration;
use yaml_rust::YamlLoader;

#[derive(Debug, Serialize, Deserialize)]
pub enum SnapshotManagement {
    /// Always render the latest snapshot.
    Single,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Hostname of the machine running the controller.
    pub server_hostname: String,
    /// Virtual video channel to listen to.
    pub video_channel: u64,
    /// UNUSED - preserved until client machines are updated
    pub render_delay: Duration,
    /// UNUSED - preserved until client machines are updated
    pub snapshot_management: SnapshotManagement,
    /// UNUSED - preserved until client machines are updated.
    pub timesync_interval: Duration,
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
    /// Log at debug level?  This option is ignored when running in remote mode.
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
        log_level_debug: bool,
    ) -> ClientConfig {
        let (x_resolution, y_resolution) = resolution;

        ClientConfig {
            server_hostname: host,
            video_channel,
            render_delay: Default::default(),
            timesync_interval: Default::default(),
            x_resolution,
            y_resolution,
            fullscreen,
            capture_mouse,
            critical_size: f64::from(cmp::min(x_resolution, y_resolution)),
            thickness_scale: 0.5,
            x_center: f64::from(x_resolution / 2),
            y_center: f64::from(y_resolution / 2),
            transformation,
            log_level_debug,
            snapshot_management: SnapshotManagement::Single,
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

        Ok(ClientConfig::new(
            video_channel,
            host,
            (x_resolution, y_resolution),
            flag("fullscreen", "Bad fullscreen flag.")?,
            flag("capture_mouse", "Bad mouse capture flag.")?,
            transformation,
            flag("log_level_debug", "Bad log level flag.")?,
        ))
    }
}

pub type Resolution = (u32, u32);
