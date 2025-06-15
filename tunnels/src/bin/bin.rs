use anyhow::Result;
use io::Write;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env::current_dir, fs::create_dir_all, io, path::PathBuf};
use tunnels::audio::prompt_audio;
use tunnels::midi::list_ports;
use tunnels::midi::prompt_midi;
use tunnels::midi_controls::Device as MidiDevice;
use tunnels::osc::Device as OscDevice;
use tunnels::osc::DeviceSpec as OscDeviceSpec;
use tunnels::show::Show;
use tunnels::test_mode::{all_video_outputs, noise, stress, TestModeSetup};
use tunnels_lib::prompt::prompt_bool;
use tunnels_lib::prompt::prompt_port;
use tunnels_lib::prompt::read_string;

/// This is approximately 240 fps, implying a worst-case client render latency of
/// essentially this value.
const RENDER_INTERVAL: Duration = Duration::from_nanos(16666667 / 4);

fn main() -> Result<()> {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())?;
    let (inputs, outputs) = list_ports()?;

    let test_mode = prompt_test_mode()?;

    let midi_devices = if test_mode.is_some() {
        Vec::new()
    } else {
        prompt_midi(&inputs, &outputs, MidiDevice::all())?
    };

    let osc_devices = if test_mode.is_some() {
        Vec::new()
    } else {
        prompt_osc()?
    };

    let audio_input_device = if test_mode.is_some() {
        None
    } else {
        prompt_audio()?
    };

    let run_clock_service = if test_mode.is_some() {
        false
    } else {
        prompt_bool("Run clock publisher service?")?
    };

    let paths = if test_mode.is_some() {
        LoadSaveConfig {
            load_path: None,
            save_path: None,
        }
    } else {
        prompt_load_save()?
    };

    let mut show = Show::new(
        midi_devices,
        osc_devices,
        audio_input_device,
        run_clock_service,
        paths.save_path,
    )?;

    if let Some(setup_test) = test_mode {
        show.test_mode(setup_test);
    } else if let Some(load_path) = paths.load_path {
        show.load(&load_path)?;
    }

    show.run(RENDER_INTERVAL)
}

/// Prompt the user to optionally configure a test mode.
fn prompt_test_mode() -> Result<Option<TestModeSetup>> {
    if !prompt_bool("Output test mode?")? {
        return Ok(None);
    }
    Ok(loop {
        print!("Select test mode ('video_outs', 'stress', 'noise'): ");
        io::stdout().flush()?;
        match &read_string()?[..] {
            "video_outs" => break Some(all_video_outputs),
            "stress" => break Some(stress),
            "noise" => break Some(noise),
            _ => (),
        }
    })
}

/// Prompt the user to configure OSC devices.
fn prompt_osc() -> Result<Vec<OscDeviceSpec>> {
    let mut devices = Vec::new();

    if prompt_bool("Use OSC color palette source?")? {
        let port = prompt_port()?;
        devices.push(OscDeviceSpec {
            device: OscDevice::PaletteController,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port),
        })
    }

    if prompt_bool("Use OSC position source?")? {
        let port = prompt_port()?;
        devices.push(OscDeviceSpec {
            device: OscDevice::PositionController,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port),
        })
    }

    Ok(devices)
}

struct LoadSaveConfig {
    load_path: Option<PathBuf>,
    save_path: Option<PathBuf>,
}

/// Save and load shows from this relative directory.
const SHOW_DIR: &str = "saved_shows";

/// Prompt the user for show load and/or save paths.
fn prompt_load_save() -> Result<LoadSaveConfig> {
    let mut cfg = LoadSaveConfig {
        load_path: None,
        save_path: None,
    };
    let save_dir = current_dir()?.join(SHOW_DIR);
    if prompt_bool("Open saved show?")? {
        let mut name = String::new();
        while name.is_empty() {
            print!("Open this show: ");
            io::stdout().flush()?;
            name = read_string()?;
        }
        let path = save_dir.join(name);
        cfg.load_path = Some(path.clone());
        cfg.save_path = Some(path);
    } else if prompt_bool("Creating new show; save?")? {
        let mut name = String::new();
        while name.is_empty() {
            print!("Name this show: ");
            io::stdout().flush()?;
            name = read_string()?;
        }
        cfg.save_path = Some(save_dir.join(name));
        create_dir_all(save_dir)?;
    }
    Ok(cfg)
}
