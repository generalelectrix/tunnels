use yaml_rust::YamlLoader;
use std::fs::File;
use std::io::Read;
use std::{env, cmp};
use std::time::Duration;

pub struct ClientConfig {
    pub server_hostname: String,
    pub video_channel: String,
    pub render_delay: f64, // milliseconds
    pub timesync_interval: Duration,
    pub x_resolution: u32,
    pub y_resolution: u32,
    pub anti_alias: bool,
    pub alpha_blend: bool,
    pub fullscreen: bool,
    pub critical_size: f64,
    pub thickness_scale: f64,
    pub x_center: f64,
    pub y_center: f64
}

/// Parses first command line arg as an integer video channel.
/// Parses second command line arg as path to a yaml config file.
/// Loads, parses, and returns the config.
/// Panics if something goes wrong.
pub fn config_from_command_line() -> ClientConfig {
    let video_channel_str = env::args().nth(1).expect("No video channel provided.");

    // Parse video channel as an int and blow up if it isn't one.
    let video_channel: u64 = video_channel_str.parse().unwrap();
    // Back into string to construct the channel filter arg.
    let channel_filter_str = video_channel.to_string();

    let config_path = env::args().nth(2).expect("No config path arg provided.");
    let mut config_file = File::open(config_path).unwrap();
    let mut config_file_string = String::new();
    config_file.read_to_string(&mut config_file_string).unwrap();
    let docs = YamlLoader::load_from_str(&config_file_string).unwrap();
    let cfg = &docs[0];
    let x_resolution = cfg["x_resolution"].as_i64().expect("Bad x resolution.") as u32;
    let y_resolution = cfg["y_resolution"].as_i64().expect("Bad y resolution.") as u32;
    let host = cfg["server_hostname"].as_str().unwrap().trim().to_string();
    let timesync_interval = Duration::from_millis(
        cfg["timesync_interval"].as_i64().expect("Bad timesync_interval.") as u64);

    println!("Running on video channel {}.", channel_filter_str);
    ClientConfig {
        server_hostname: host,
        video_channel: channel_filter_str,
        render_delay: cfg["render_delay"].as_i64().expect("Bad render delay.") as f64,
        timesync_interval,
        x_resolution,
        y_resolution,
        anti_alias: cfg["anti_alias"].as_bool().expect("Bad anti-alias flag."),
        fullscreen: cfg["fullscreen"].as_bool().expect("Bad fullscreen flag."),
        critical_size: cmp::min(x_resolution, y_resolution) as f64,
        thickness_scale: 0.5,
        x_center: (x_resolution / 2) as f64,
        y_center: (y_resolution / 2) as f64,
        alpha_blend: cfg["alpha_blend"].as_bool().expect("Bad alpha_blend flag.")
    }
}
