//! Loading and parsing client configurations.
use crate::transform::{Transform, TransformDirection};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::cmp;
use std::fs::File;
use std::io::Read;
use yaml_rust::YamlLoader;

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
    pub artnet_node: bool,
    /// Log at debug level?
    pub log_level_debug: bool,
    /// Let a figure larger than the startup mesh table anticipated be given a
    /// mesh matched to its size, built the first time it is drawn.
    ///
    /// Off, a figure that wants more density than the table holds is drawn
    /// with the finest mesh the table does hold, and nothing is ever built
    /// during a show. On, the two finer densities become available and are
    /// built per figure on first use — a pause of about 7 ms typically and 17
    /// at worst for the first, 28 and 64 for the second.
    ///
    /// It defaults off because the question it answers is whether the extra
    /// density is visible at all, and that is a judgement to make by eye
    /// rather than from the triangle count.
    ///
    /// Where the knobs cross those thresholds, at 1920x1200 — `critical_size`
    /// is the smaller dimension, so 1200:
    ///
    /// - the default size of 0.5 sits inside the table, as it does at 1080;
    /// - **size 0.528** is where a figure first wants more, with aspect ratio
    ///   at its own default;
    /// - the second threshold needs an extent of 1.056, which the size knob
    ///   cannot reach alone since it stops at 1.0 — it takes aspect ratio
    ///   above 0.528 as well.
    ///
    /// Those two lists line up in a way worth knowing before turning this on:
    /// **the threshold that is easy to reach is the one that costs nothing,
    /// and the one that costs enough to see is off the casual path.** A nudge
    /// of the size knob crosses the first, and pays two frames at worst. The
    /// second takes two knobs deliberately, and pays about eight — a
    /// fifteenth of a second, which is visible. So the risk is not a knob
    /// brushed past by accident.
    pub refine_large_figures: bool,
    /// How many pixels a filled figure's mesh triangles should span on screen.
    ///
    /// Triangle count goes as the inverse square of this, so it is the
    /// strongest lever there is on what a figure costs per frame — and the one
    /// a slow projector machine has.
    pub target_px: f64,
}

/// Default for [`ClientConfig::target_px`].
///
/// A figure's mesh carries phase, not colour: colour is resolved per fragment
/// against the ramp. Phase is smooth, so the mesh only has to be fine enough
/// that a straight line approximates an arctangent or a square root over one
/// triangle, which is a far weaker requirement than approximating a hue sweep
/// would be.
pub const DEFAULT_TARGET_PX: f64 = 14.0;

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
        artnet_node: bool,
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
            refine_large_figures: false,
            target_px: DEFAULT_TARGET_PX,
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

        let mut config = ClientConfig::new(
            video_channel,
            host,
            (x_resolution, y_resolution),
            flag("fullscreen", "Bad fullscreen flag.")?,
            flag("capture_mouse", "Bad mouse capture flag.")?,
            transformation,
            // An absent key leaves the node off, so a config written before
            // clients could serve one still loads.
            cfg["artnet_node"].as_bool().unwrap_or(false),
            flag("log_level_debug", "Bad log level flag.")?,
        );
        // An absent key leaves the default, so a config written before figures
        // existed still loads. A machine too slow for them turns this up.
        if let Some(target_px) = cfg["target_px"].as_f64() {
            config.target_px = target_px;
        }
        // Absent means off, which is what a config written before figures
        // existed should mean.
        config.refine_large_figures = cfg["refine_large_figures"].as_bool().unwrap_or(false);
        Ok(config)
    }
}

pub type Resolution = (u32, u32);
