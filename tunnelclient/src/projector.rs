//! Projector control service — advertises over DNS-SD, accepts req/rep
//! commands from the console, and translates them into ESC/VP21 serial
//! commands sent to the attached Epson projector.
//!
//! The service starts without a serial port. The console can list available
//! ports and connect/disconnect remotely.

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Result};
use log::{error, info, warn};
use serialport::SerialPort;
use tunnels_lib::projector::{
    PowerState, ProjectorRequest, ProjectorResponse, ProjectorResponseOk, ProjectorStatus,
};
use zero_configure::req_rep::run_service_req_rep;

const SERVICE_NAME: &str = "tunnelprojector";
const PORT: u16 = 16000;

/// Mutable serial port state — None when not connected.
struct PortState {
    port: Option<Box<dyn SerialPort>>,
    port_path: Option<String>,
}

/// Spawn the projector control service in a background thread.
///
/// The service starts immediately and advertises via DNS-SD, even without a
/// serial port connected. The console can list available ports and connect
/// remotely.
pub fn spawn_projector_service() -> Result<()> {
    thread::Builder::new()
        .name("projector_control".to_string())
        .spawn(move || {
            let state = Mutex::new(PortState {
                port: None,
                port_path: None,
            });

            info!("Projector control service starting (no serial port connected)");
            if let Err(e) =
                run_service_req_rep(SERVICE_NAME, PORT, |buf| handle_request(buf, &state))
            {
                error!("Projector service error: {e}");
            }
        })?;

    Ok(())
}

fn handle_request(buffer: &[u8], state: &Mutex<PortState>) -> Vec<u8> {
    let response = handle_request_inner(buffer, state);
    rmp_serde::to_vec(&response).unwrap_or_else(|e| {
        error!("Failed to serialize projector response: {e}");
        Vec::new()
    })
}

fn handle_request_inner(buffer: &[u8], state: &Mutex<PortState>) -> ProjectorResponse {
    let request: ProjectorRequest =
        rmp_serde::from_slice(buffer).map_err(|e| format!("Failed to deserialize request: {e}"))?;

    let mut state = state.lock().unwrap();

    match request {
        ProjectorRequest::GetStatus => {
            let mut status = ProjectorStatus {
                available_ports: list_serial_ports(),
                connected_port: state.port_path.clone(),
                ..Default::default()
            };

            if let Some(ref mut port) = state.port {
                match query_projector_status(&mut **port) {
                    Ok(pj) => {
                        status.power = pj.power;
                        status.av_mute = pj.av_mute;
                        status.eco_mode = pj.eco_mode;
                        status.lamp_hours = pj.lamp_hours;
                        status.color_mode = pj.color_mode;
                        status.error_code = pj.error_code;
                    }
                    Err(e) => {
                        warn!("Failed to query projector: {e}");
                    }
                }
            }

            Ok(ProjectorResponseOk::Status(status))
        }
        ProjectorRequest::Connect(path) => {
            info!("Connecting to serial port: {path}");
            let port = open_serial_port(&path).map_err(|e| e.to_string())?;
            state.port = Some(port);
            state.port_path = Some(path);
            Ok(ProjectorResponseOk::Ok)
        }
        ProjectorRequest::Disconnect => {
            if let Some(ref path) = state.port_path {
                info!("Disconnecting from serial port: {path}");
            }
            state.port = None;
            state.port_path = None;
            Ok(ProjectorResponseOk::Ok)
        }
        ProjectorRequest::SetPower(on) => {
            let port = require_port(&mut state)?;
            let cmd = if on { "PWR ON" } else { "PWR OFF" };
            send_command(port.as_mut(), cmd)
                .map(|_| ProjectorResponseOk::Ok)
                .map_err(|e| e.to_string())
        }
        ProjectorRequest::SetAvMute(on) => {
            let port = require_port(&mut state)?;
            let cmd = if on { "MUTE ON" } else { "MUTE OFF" };
            send_command(port.as_mut(), cmd)
                .map(|_| ProjectorResponseOk::Ok)
                .map_err(|e| e.to_string())
        }
        ProjectorRequest::SetEcoMode(eco) => {
            let port = require_port(&mut state)?;
            let cmd = if eco { "LUMINANCE 01" } else { "LUMINANCE 00" };
            send_command(port.as_mut(), cmd)
                .map(|_| ProjectorResponseOk::Ok)
                .map_err(|e| e.to_string())
        }
    }
}

fn require_port(state: &mut PortState) -> Result<&mut Box<dyn SerialPort>, String> {
    state
        .port
        .as_mut()
        .ok_or_else(|| "No serial port connected. Use Connect first.".to_string())
}

fn open_serial_port(path: &str) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(path, 9600)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_secs(45))
        .open()?;
    Ok(port)
}

/// Query all projector state fields over ESC/VP21.
fn query_projector_status(port: &mut dyn SerialPort) -> Result<ProjectorStatus, String> {
    let power = query_power(port).map_err(|e| format!("PWR? failed: {e}"))?;

    // When the projector is not fully on, most queries return ERR.
    if power != PowerState::On {
        return Ok(ProjectorStatus {
            power,
            ..Default::default()
        });
    }

    let av_mute = send_command(port, "MUTE?")
        .map(|r| r == "ON")
        .unwrap_or_else(|e| {
            warn!("MUTE? failed: {e}");
            false
        });

    let eco_mode = send_command(port, "LUMINANCE?")
        .map(|r| r == "01")
        .unwrap_or_else(|e| {
            warn!("LUMINANCE? failed: {e}");
            false
        });

    let lamp_hours = send_command(port, "LAMP?")
        .ok()
        .and_then(|r| r.parse::<u32>().ok())
        .unwrap_or(0);

    let error_code = send_command(port, "ERR?")
        .ok()
        .and_then(|r| u8::from_str_radix(&r, 16).ok())
        .unwrap_or(0);

    let color_mode = send_command(port, "CMODE?")
        .ok()
        .map(|code| color_mode_name(&code));

    Ok(ProjectorStatus {
        power,
        av_mute,
        eco_mode,
        lamp_hours,
        color_mode,
        error_code,
        ..Default::default()
    })
}

fn query_power(port: &mut dyn SerialPort) -> Result<PowerState> {
    let response = send_command(port, "PWR?")?;
    Ok(match response.as_str() {
        "00" => PowerState::Off,
        "01" => PowerState::On,
        "02" => PowerState::WarmingUp,
        "03" => PowerState::CoolingDown,
        "04" => PowerState::Standby,
        "05" => PowerState::Error,
        other => {
            warn!("Unknown PWR? response: {other}");
            PowerState::Error
        }
    })
}

/// Send an ESC/VP21 command and read the response.
///
/// Set commands return an empty string on success (just the `:` delimiter).
/// Query commands return the value before the `:` delimiter.
pub fn send_command(port: &mut dyn SerialPort, cmd: &str) -> Result<String> {
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

/// Map an Epson CMODE? hex code to a human-readable name.
pub fn color_mode_name(code: &str) -> String {
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
        other => return format!("Unknown ({other})"),
    }
    .to_string()
}

/// List serial ports that look like USB-to-serial adapters.
pub fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.port_name.contains("usbserial")
                || p.port_name.contains("usbmodem")
                || p.port_name.contains("ttyUSB")
                || p.port_name.contains("ttyACM")
        })
        .map(|p| p.port_name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mode_known_codes() {
        assert_eq!(color_mode_name("06"), "Dynamic");
        assert_eq!(color_mode_name("04"), "Presentation");
        assert_eq!(color_mode_name("01"), "sRGB");
    }

    #[test]
    fn color_mode_unknown_code() {
        assert_eq!(color_mode_name("FF"), "Unknown (FF)");
    }

    #[test]
    fn service_name_registers() {
        let stop =
            zero_configure::bare::register_service(SERVICE_NAME, 0).expect("should register");
        stop();
    }
}
