//! Interactive tester for ESC/VP21 projector control over serial.
//!
//! Usage:
//!   projector-test                    # list available serial ports
//!   projector-test <port>             # connect and run interactive commands
//!   projector-test <port> status      # query and print full status, then exit
//!   projector-test <port> raw <cmd>   # send a raw ESC/VP21 command
//!
//! Examples:
//!   projector-test /dev/tty.usbserial-1420
//!   projector-test /dev/tty.usbserial-1420 status
//!   projector-test /dev/tty.usbserial-1420 raw "PWR?"
//!   projector-test /dev/tty.usbserial-1420 raw "PWR ON"

use anyhow::{bail, Result};
use std::io::{self, BufRead, Write};
use std::time::Duration;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        return list_ports();
    }

    let port_path = &args[1];
    let mut port = open_port(port_path)?;
    println!("Connected to {port_path} (9600 8N1, 45s timeout)");

    if args.len() >= 3 {
        match args[2].as_str() {
            "status" => return print_status(&mut *port),
            "raw" => {
                if args.len() < 4 {
                    bail!("Usage: projector-test <port> raw <command>");
                }
                let cmd = &args[3];
                let response = send_command(&mut *port, cmd)?;
                if response.is_empty() {
                    println!("OK (no response body)");
                } else {
                    println!("{response}");
                }
                return Ok(());
            }
            other => bail!("Unknown subcommand: {other}. Use 'status' or 'raw'."),
        }
    }

    // Interactive REPL
    println!();
    println!("Interactive mode. Commands:");
    println!("  on / off         — power on/off");
    println!("  mute / unmute    — A/V mute on/off");
    println!("  eco / standard   — lamp mode");
    println!("  status           — query full status");
    println!("  raw <cmd>        — send raw ESC/VP21 (e.g. 'raw PWR?')");
    println!("  ports            — list serial ports");
    println!("  quit             — exit");
    println!();

    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match line {
            "quit" | "exit" | "q" => break,
            "on" => match send_command(&mut *port, "PWR ON") {
                Ok(_) => println!("Power ON sent."),
                Err(e) => println!("Error: {e}"),
            },
            "off" => match send_command(&mut *port, "PWR OFF") {
                Ok(_) => println!("Power OFF sent."),
                Err(e) => println!("Error: {e}"),
            },
            "mute" => match send_command(&mut *port, "MUTE ON") {
                Ok(_) => println!("A/V Mute ON."),
                Err(e) => println!("Error: {e}"),
            },
            "unmute" => match send_command(&mut *port, "MUTE OFF") {
                Ok(_) => println!("A/V Mute OFF."),
                Err(e) => println!("Error: {e}"),
            },
            "eco" => match send_command(&mut *port, "LUMINANCE 01") {
                Ok(_) => println!("ECO mode set."),
                Err(e) => println!("Error: {e}"),
            },
            "standard" => match send_command(&mut *port, "LUMINANCE 00") {
                Ok(_) => println!("Standard mode set."),
                Err(e) => println!("Error: {e}"),
            },
            "status" => {
                if let Err(e) = print_status(&mut *port) {
                    println!("Error: {e}");
                }
            }
            "ports" => {
                if let Err(e) = list_ports() {
                    println!("Error: {e}");
                }
            }
            _ if line.starts_with("raw ") => {
                let cmd = &line[4..];
                match send_command(&mut *port, cmd) {
                    Ok(r) if r.is_empty() => println!("OK"),
                    Ok(r) => println!("{r}"),
                    Err(e) => println!("Error: {e}"),
                }
            }
            _ => println!("Unknown command: {line}"),
        }
    }

    Ok(())
}

fn list_ports() -> Result<()> {
    let ports = serialport::available_ports()?;

    if ports.is_empty() {
        println!("No serial ports found.");
        return Ok(());
    }

    println!("Available serial ports:");
    for port in &ports {
        let info = match &port.port_type {
            serialport::SerialPortType::UsbPort(usb) => {
                let mfg = usb.manufacturer.as_deref().unwrap_or("?");
                let product = usb.product.as_deref().unwrap_or("?");
                format!(
                    "  USB: {mfg} - {product} (VID:{:04x} PID:{:04x})",
                    usb.vid, usb.pid
                )
            }
            serialport::SerialPortType::BluetoothPort => "  Bluetooth".to_string(),
            serialport::SerialPortType::PciPort => "  PCI".to_string(),
            serialport::SerialPortType::Unknown => "".to_string(),
        };
        println!("  {}{info}", port.port_name);
    }
    Ok(())
}

fn open_port(path: &str) -> Result<Box<dyn serialport::SerialPort>> {
    let port = serialport::new(path, 9600)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_secs(45))
        .open()?;
    Ok(port)
}

fn send_command(port: &mut dyn serialport::SerialPort, cmd: &str) -> Result<String> {
    port.write_all(format!("{cmd}\r").as_bytes())?;
    port.flush()?;

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        port.read_exact(&mut byte)?;
        if byte[0] == b':' {
            break;
        }
        buf.push(byte[0]);
    }

    let response = String::from_utf8(buf)?.trim().to_string();
    if response == "ERR" {
        bail!("Projector returned ERR for command: {cmd}");
    }
    Ok(response)
}

fn print_status(port: &mut dyn serialport::SerialPort) -> Result<()> {
    // Power state
    let power = send_command(port, "PWR?")?;
    let power_label = match power.as_str() {
        "00" => "Off (standby, network off)",
        "01" => "On",
        "02" => "Warming up",
        "03" => "Cooling down",
        "04" => "Standby (network on)",
        "05" => "Error (abnormal standby)",
        _ => "Unknown",
    };
    println!("Power:      {power_label} ({power})");

    // Only query the rest if the projector is fully on.
    if power != "01" {
        println!("(Projector not fully on — remaining queries skipped)");
        return Ok(());
    }

    // A/V Mute
    match send_command(port, "MUTE?") {
        Ok(r) => println!("A/V Mute:   {r}"),
        Err(e) => println!("A/V Mute:   (error: {e})"),
    }

    // Lamp mode
    match send_command(port, "LUMINANCE?") {
        Ok(r) => {
            let label = if r == "01" { "ECO" } else { "Standard" };
            println!("Lamp Mode:  {label} ({r})");
        }
        Err(e) => println!("Lamp Mode:  (error: {e})"),
    }

    // Color mode
    match send_command(port, "CMODE?") {
        Ok(r) => {
            let name = color_mode_name(&r);
            println!("Color Mode: {name} ({r})");
        }
        Err(e) => println!("Color Mode: (error: {e})"),
    }

    // Lamp hours
    match send_command(port, "LAMP?") {
        Ok(r) => println!("Lamp Hours: {r}"),
        Err(e) => println!("Lamp Hours: (error: {e})"),
    }

    // Source
    match send_command(port, "SOURCE?") {
        Ok(r) => {
            let name = source_name(&r);
            println!("Source:     {name} ({r})");
        }
        Err(e) => println!("Source:     (error: {e})"),
    }

    // Error status
    match send_command(port, "ERR?") {
        Ok(r) => {
            let label = if r == "00" { "None" } else { &r };
            println!("Error:      {label}");
        }
        Err(e) => println!("Error:      (error: {e})"),
    }

    // Serial number
    match send_command(port, "SNO?") {
        Ok(r) => println!("Serial:     {r}"),
        Err(e) => println!("Serial:     (error: {e})"),
    }

    Ok(())
}

fn color_mode_name(code: &str) -> &str {
    match code {
        "00" => "Auto",
        "01" => "sRGB",
        "03" => "Presentation 2",
        "04" => "Presentation",
        "05" => "Theatre",
        "06" => "Dynamic",
        "07" => "Natural",
        "08" => "Sports",
        "09" => "Theatre Black 1",
        "0A" => "Theatre Black 2",
        "0C" => "Bright Cinema",
        "0D" => "Game",
        "10" => "Custom",
        "11" => "Blackboard",
        "12" => "Whiteboard",
        "14" => "Photo",
        "15" => "Cinema",
        _ => "Unknown",
    }
}

fn source_name(code: &str) -> &str {
    match code {
        "10" => "Computer 1 (VGA)",
        "20" => "Computer 2",
        "30" => "HDMI 1",
        "A0" => "HDMI 2",
        "41" => "Video",
        "52" => "USB",
        "53" => "LAN",
        "56" => "WiFi Direct",
        _ => "Unknown",
    }
}
