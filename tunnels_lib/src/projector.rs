//! Protocol types for projector control over the req/rep service.
//!
//! These types are projector-agnostic. The ESC/VP21 serial implementation in
//! `tunnelclient` is one backend; changing the projector brand or control
//! interface only requires changing the serial I/O layer.

use serde::{Deserialize, Serialize};

/// Command sent from the console to a render client's projector service.
#[derive(Serialize, Deserialize, Debug)]
pub enum ProjectorRequest {
    /// Query the full current state, including available serial ports.
    GetStatus,
    /// Connect to a serial port by path (e.g. "/dev/tty.usbserial-1420").
    Connect(String),
    /// Disconnect from the current serial port.
    Disconnect,
    /// Turn projector power on or off.
    SetPower(bool),
    /// Enable or disable A/V mute (blanks image and mutes speaker).
    SetAvMute(bool),
    /// Set lamp power mode: `true` = ECO, `false` = Standard.
    SetEcoMode(bool),
}

/// Response from the render client's projector service.
pub type ProjectorResponse = Result<ProjectorResponseOk, String>;

/// Successful response payload.
#[derive(Serialize, Deserialize, Debug)]
pub enum ProjectorResponseOk {
    /// Acknowledgment that a Set command was accepted.
    Ok,
    /// Full projector status snapshot.
    Status(ProjectorStatus),
}

/// Snapshot of the projector's current state.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProjectorStatus {
    /// Serial ports available on this machine.
    pub available_ports: Vec<String>,
    /// Currently connected serial port path, if any.
    pub connected_port: Option<String>,
    /// Projector state — only populated when connected and projector responds.
    pub power: PowerState,
    pub av_mute: bool,
    pub eco_mode: bool,
    pub lamp_hours: u32,
    pub color_mode: Option<String>,
    pub error_code: u8,
}

/// Projector power state.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub enum PowerState {
    #[default]
    Off,
    On,
    WarmingUp,
    CoolingDown,
    Standby,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let cases: Vec<ProjectorRequest> = vec![
            ProjectorRequest::GetStatus,
            ProjectorRequest::Connect("/dev/tty.usbserial-1420".to_string()),
            ProjectorRequest::Disconnect,
            ProjectorRequest::SetPower(true),
            ProjectorRequest::SetPower(false),
            ProjectorRequest::SetAvMute(true),
            ProjectorRequest::SetAvMute(false),
            ProjectorRequest::SetEcoMode(true),
            ProjectorRequest::SetEcoMode(false),
        ];
        for req in cases {
            let bytes = rmp_serde::to_vec(&req).unwrap();
            let _: ProjectorRequest = rmp_serde::from_slice(&bytes).unwrap();
        }
    }

    #[test]
    fn response_ok_round_trip() {
        let ok: ProjectorResponse = Ok(ProjectorResponseOk::Ok);
        let bytes = rmp_serde::to_vec(&ok).unwrap();
        let de: ProjectorResponse = rmp_serde::from_slice(&bytes).unwrap();
        assert!(de.is_ok());
    }

    #[test]
    fn response_status_round_trip() {
        let status = ProjectorStatus {
            available_ports: vec!["/dev/tty.usbserial-1420".to_string()],
            connected_port: Some("/dev/tty.usbserial-1420".to_string()),
            power: PowerState::On,
            av_mute: false,
            eco_mode: true,
            lamp_hours: 1234,
            color_mode: Some("Presentation".to_string()),
            error_code: 0,
        };
        let resp: ProjectorResponse = Ok(ProjectorResponseOk::Status(status));
        let bytes = rmp_serde::to_vec(&resp).unwrap();
        let de: ProjectorResponse = rmp_serde::from_slice(&bytes).unwrap();
        match de.unwrap() {
            ProjectorResponseOk::Status(s) => {
                assert_eq!(s.available_ports.len(), 1);
                assert_eq!(s.connected_port.as_deref(), Some("/dev/tty.usbserial-1420"));
                assert_eq!(s.power, PowerState::On);
                assert!(!s.av_mute);
                assert!(s.eco_mode);
                assert_eq!(s.lamp_hours, 1234);
                assert_eq!(s.color_mode.as_deref(), Some("Presentation"));
                assert_eq!(s.error_code, 0);
            }
            ProjectorResponseOk::Ok => panic!("Expected Status"),
        }
    }

    #[test]
    fn response_error_round_trip() {
        let err: ProjectorResponse = Err("serial port timeout".to_string());
        let bytes = rmp_serde::to_vec(&err).unwrap();
        let de: ProjectorResponse = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(de.unwrap_err(), "serial port timeout");
    }
}
