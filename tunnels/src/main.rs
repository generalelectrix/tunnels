mod animation;
mod beam;
mod beam_store;
mod clock;
mod clock_bank;
mod device;
mod look;
mod master_ui;
mod midi;
mod midi_controls;
mod mixer;
mod send;
mod show;
mod test_mode;
mod timesync;
mod tunnel;
mod waveforms;

use device::Device;
use io::Write;
use midi::{list_ports, DeviceSpec};
use show::Show;
use simplelog::{Config as LogConfig, LevelFilter, SimpleLogger};
use std::io;
use std::{error::Error, time::Duration};
use test_mode::{all_video_outputs, stress, TestModeSetup};

fn main() -> Result<(), Box<dyn Error>> {
    SimpleLogger::init(LevelFilter::Info, LogConfig::default())?;
    let (inputs, outputs) = list_ports()?;

    let test_mode = prompt_test_mode()?;

    let devices = if test_mode.is_some() {
        Vec::new()
    } else {
        prompt_midi(&inputs, &outputs)?
    };

    let mut show = Show::new(devices)?;

    if let Some(setup_test) = test_mode {
        show.test_mode(setup_test);
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
) -> Result<Vec<DeviceSpec>, Box<dyn Error>> {
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

    add_device(Device::TouchOsc)?;
    add_device(Device::AkaiApc40)?;
    add_device(Device::BehringerCmdMM1)?;
    add_device(Device::AkaiApc20)?;

    Ok(devices)
}

/// Prompt the user to select input and output ports for a device.
fn prompt_input_output(
    device: Device,
    input_ports: &Vec<String>,
    output_ports: &Vec<String>,
) -> Result<DeviceSpec, Box<dyn Error>> {
    let input_port_name = prompt_indexed_value("Input port:", input_ports)?;
    let output_port_name = prompt_indexed_value("Output port:", output_ports)?;
    Ok(DeviceSpec {
        device,
        input_port_name,
        output_port_name,
    })
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

/// Prompt the user to answer a yes or no question.
fn prompt_bool(msg: &str) -> Result<bool, Box<dyn Error>> {
    Ok(loop {
        print!("{} y/n: ", msg);
        io::stdout().flush()?;
        let input = read_string()?;
        if let Some(first_char) = input.chars().nth(0) {
            match first_char {
                'y' | 'Y' => break true,
                'n' | 'N' => break false,
                _ => (),
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
