use yaml_rust::YamlLoader;
use std::fs::File;
use std::io::Read;
use std::{env, cmp};

pub struct ClientConfig {
    pub server_hostname: String,
    pub render_delay: u64, // milliseconds
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

/// Parses first command line arg as path to a yaml config file.
/// Loads, parses, and returns the config.
/// Panics if something goes wrong.
pub fn config_from_command_line() -> ClientConfig {
    let config_path = env::args().nth(1).expect("No config path arg provided.");
    let mut config_file = File::open(config_path).unwrap();
    let mut config_file_string = String::new();
    config_file.read_to_string(&mut config_file_string).unwrap();
    let docs = YamlLoader::load_from_str(&config_file_string).unwrap();
    let cfg = &docs[0];
    let x_resolution = cfg["x_resolution"].as_i64().unwrap() as u32;
    let y_resolution = cfg["y_resolution"].as_i64().unwrap() as u32;
    let host = cfg["server_hostname"].as_str().unwrap().trim().to_string();
    ClientConfig {
        server_hostname: host,
        render_delay: cfg["render_delay"].as_i64().unwrap() as u64,
        x_resolution: x_resolution,
        y_resolution: y_resolution,
        anti_alias: cfg["anti_alias"].as_bool().unwrap(),
        fullscreen: cfg["fullscreen"].as_bool().unwrap(),
        critical_size: cmp::min(x_resolution, y_resolution) as f64,
        thickness_scale: 0.5,
        x_center: (x_resolution / 2) as f64,
        y_center: (y_resolution / 2) as f64,
        alpha_blend: cfg["alpha_blend"].as_bool().unwrap()
    }
}