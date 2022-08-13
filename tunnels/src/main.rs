mod animation;
mod beam;
mod beam_store;
mod clock;
mod clock_bank;
mod control;
mod look;
mod master_ui;
mod midi;
mod midi_controls;
mod mixer;
mod osc;
mod palette;
mod send;
mod show;
mod test_mode;
mod timesync;
mod tunnel;
mod waveforms;

use io::Write;
use midi::{list_ports, DeviceSpec as MidiDeviceSpec};
use midi_controls::Device as MidiDevice;
use osc::Device as OscDevice;
use osc::DeviceSpec as OscDeviceSpec;
use show::Show;
use simple_error::SimpleError;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::{env::current_dir, fs::create_dir_all, io, path::PathBuf};
use std::{error::Error, time::Duration};
use test_mode::{all_video_outputs, stress, TestModeSetup};

fn main() -> Result<(), Box<dyn Error>> {
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

    let mut show = Show::new(midi_devices, osc_devices)?;

    if let Some(setup_test) = test_mode {
        show.test_mode(setup_test);
    } else {
        let paths = prompt_load_save()?;
        show.save_path = paths.save_path;
        if let Some(load_path) = paths.load_path {
            show.load(&load_path)?;
        }
    }

    show.run(Duration::from_micros(16667))
}

/// Prompt the user to optionally configure a test mode.
fn prompt_test_mode() -> Result<Option<TestModeSetup>, Box<dyn Error>> {
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
) -> Result<Vec<MidiDeviceSpec>, Box<dyn Error>> {
    let mut devices = Vec::new();
    println!("Available devices:");
    for (i, port) in input_ports.iter().enumerate() {
        println!("{}: {}", i, port);
    }
    for (i, port) in output_ports.iter().enumerate() {
        println!("{}: {}", i, port);
    }
    println!();

    let mut add_device = |device| -> Result<(), Box<dyn Error>> {
        if prompt_bool(&format!("Use {}?", device))? {
            devices.push(prompt_input_output(device, input_ports, output_ports)?);
        }
        Ok(())
    };

    add_device(MidiDevice::TouchOsc)?;
    add_device(MidiDevice::AkaiApc40)?;
    add_device(MidiDevice::BehringerCmdMM1)?;
    add_device(MidiDevice::AkaiApc20)?;

    Ok(devices)
}

/// Prompt the user to select input and output ports for a device.
fn prompt_input_output(
    device: MidiDevice,
    input_ports: &Vec<String>,
    output_ports: &Vec<String>,
) -> Result<MidiDeviceSpec, Box<dyn Error>> {
    let input_port_name = prompt_indexed_value("Input port:", input_ports)?;
    let output_port_name = prompt_indexed_value("Output port:", output_ports)?;
    Ok(MidiDeviceSpec {
        device,
        input_port_name,
        output_port_name,
    })
}

/// Prompt the user to configure OSC devices.
fn prompt_osc() -> Result<Vec<OscDeviceSpec>, Box<dyn Error>> {
    let mut devices = Vec::new();

    if prompt_bool("Use OSC color palette source?")? {
        let port = prompt_port()?;
        devices.push(OscDeviceSpec {
            device: OscDevice::PaletteController,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port),
        })
    }

    Ok(devices)
}

/// Prompt the user for a unsigned numeric index.
fn prompt_indexed_value<T: Clone>(msg: &str, options: &Vec<T>) -> Result<T, Box<dyn Error>> {
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
const SHOW_DIR: &'static str = "saved_shows";

/// Prompt the user for show load and/or save paths.
fn prompt_load_save() -> Result<LoadSaveConfig, Box<dyn Error>> {
    let mut cfg = LoadSaveConfig {
        load_path: None,
        save_path: None,
    };
    let save_dir = current_dir()?.join(SHOW_DIR);
    if prompt_bool("Open saved show?")? {
        let mut name = String::new();
        while name.len() == 0 {
            print!("Open this show: ");
            io::stdout().flush()?;
            name = read_string()?;
        }
        let path = save_dir.join(name);
        cfg.load_path = Some(path.clone());
        cfg.save_path = Some(path);
    } else if prompt_bool("Creating new show; save?")? {
        let mut name = String::new();
        while name.len() == 0 {
            print!("Name this show: ");
            io::stdout().flush()?;
            name = read_string()?;
        }
        cfg.save_path = Some(save_dir.join(name));
        create_dir_all(save_dir)?;
    }
    Ok(cfg)
}

/// Prompt the user to answer a yes or no question.
fn prompt_bool(msg: &str) -> Result<bool, Box<dyn Error>> {
    prompt_parse(format!("{} y/n", msg).as_str(), |input| {
        input
            .chars()
            .nth(0)
            .and_then(|first_char| match first_char {
                'y' | 'Y' => Some(true),
                'n' | 'N' => Some(false),
                _ => None,
            })
            .ok_or(Box::new(SimpleError::new("Please enter yes or no.")))
    })
}

/// Prompt the user to enter a network port.
fn prompt_port() -> Result<u16, Box<dyn Error>> {
    prompt_parse("Enter a port number", |port| {
        let parsed = port.parse::<u16>()?;
        Ok(parsed)
    })
}

/// Prompt the user for input, then parse.
fn prompt_parse<T, F>(msg: &str, parse: F) -> Result<T, Box<dyn Error>>
where
    F: Fn(&str) -> Result<T, Box<dyn Error>>,
{
    Ok(loop {
        print!("{}: ", msg);
        io::stdout().flush()?;
        let input = read_string()?;
        match parse(&input) {
            Ok(v) => break v,
            Err(e) => {
                println!("{}", e);
            }
        }
    })
}

/// Read a line of input from stdin.
fn read_string() -> Result<String, Box<dyn Error>> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
