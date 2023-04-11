use anyhow::{bail, Result};
use io::Write;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env::current_dir, fs::create_dir_all, io, path::PathBuf};
use tunnels::audio::AudioInput;
use tunnels::midi::{list_ports, DeviceSpec as MidiDeviceSpec};
use tunnels::midi_controls::Device as MidiDevice;
use tunnels::osc::Device as OscDevice;
use tunnels::osc::DeviceSpec as OscDeviceSpec;
use tunnels::show::Show;
use tunnels::test_mode::{all_video_outputs, stress, TestModeSetup};
use tunnels_lib::prompt::prompt_bool;
use tunnels_lib::prompt::prompt_port;
use tunnels_lib::prompt::read_string;

fn main() -> Result<()> {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())?;
    let (inputs, outputs) = list_ports()?;

    let test_mode = prompt_test_mode()?;

    let midi_devices = if test_mode.is_some() {
        Vec::new()
    } else {
        prompt_midi(&inputs, &outputs)?
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

    show.run(Duration::from_micros(16667))
}

/// Prompt the user to optionally configure a test mode.
fn prompt_test_mode() -> Result<Option<TestModeSetup>> {
    if !prompt_bool("Output test mode?")? {
        return Ok(None);
    }
    Ok(loop {
        print!("Select test mode ('video_outs', 'stress'): ");
        io::stdout().flush()?;
        match &read_string()?[..] {
            "video_outs" => break Some(all_video_outputs),
            "stress" => break Some(stress),
            _ => (),
        }
    })
}

/// Prompt the user to configure midi devices.
fn prompt_midi(
    input_ports: &Vec<String>,
    output_ports: &Vec<String>,
) -> Result<Vec<MidiDeviceSpec>> {
    let mut devices = Vec::new();
    println!("Available devices:");
    for (i, port) in input_ports.iter().enumerate() {
        println!("{}: {}", i, port);
    }
    for (i, port) in output_ports.iter().enumerate() {
        println!("{}: {}", i, port);
    }
    println!();

    let mut add_device = |device| -> Result<()> {
        if prompt_bool(&format!("Use {}?", device))? {
            devices.push(prompt_input_output(device, input_ports, output_ports)?);
        }
        Ok(())
    };

    add_device(MidiDevice::TouchOsc)?;
    add_device(MidiDevice::AkaiApc40)?;
    add_device(MidiDevice::BehringerCmdMM1)?;
    // add_device(MidiDevice::AkaiApc20)?;

    Ok(devices)
}

/// Prompt the user to select input and output ports for a device.
fn prompt_input_output(
    device: MidiDevice,
    input_ports: &Vec<String>,
    output_ports: &Vec<String>,
) -> Result<MidiDeviceSpec> {
    let input_port_name = prompt_indexed_value("Input port:", input_ports)?;
    let output_port_name = prompt_indexed_value("Output port:", output_ports)?;
    Ok(MidiDeviceSpec {
        device,
        input_port_name,
        output_port_name,
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

/// Prompt the user to configure an audio input device.
fn prompt_audio() -> Result<Option<String>> {
    if !prompt_bool("Use audio input?")? {
        return Ok(None);
    }
    let input_devices = AudioInput::devices()?;
    if input_devices.is_empty() {
        bail!("No audio input devices found.");
    }
    println!("Available devices:");
    for (i, port) in input_devices.iter().enumerate() {
        println!("{}: {}", i, port);
    }
    prompt_indexed_value("Input audio device:", &input_devices).map(Some)
}

/// Prompt the user for a unsigned numeric index.
fn prompt_indexed_value<T: Clone>(msg: &str, options: &Vec<T>) -> Result<T> {
    Ok(loop {
        print!("{} ", msg);
        io::stdout().flush()?;
        let input = read_string()?;
        let index = match input.trim().parse::<usize>() {
            Ok(num) => num,
            Err(e) => {
                println!("{}; please enter an integer.", e);
                continue;
            }
        };
        match options.get(index) {
            Some(v) => break v.clone(),
            None => println!("Please enter a value less than {}.", options.len()),
        }
    })
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
