//! Console-side controller for discovering and commanding projector services
//! advertised by render clients.

use anyhow::Result;
use std::time::Duration;
use tunnels_lib::projector::{
    ProjectorRequest, ProjectorResponse, ProjectorResponseOk, ProjectorStatus,
};
use zero_configure::req_rep::Controller;

const SERVICE_NAME: &str = "tunnelprojector";

/// Discovers projector service instances on the LAN and sends commands to them.
pub struct ProjectorController {
    controller: Controller,
}

impl ProjectorController {
    pub fn new(timeout: Option<Duration>) -> Self {
        Self {
            controller: Controller::with_recv_timeout(SERVICE_NAME.to_string(), timeout),
        }
    }

    /// List discovered projector service instances.
    pub fn list(&self) -> Vec<String> {
        self.controller.list()
    }

    pub fn get_status(&self, name: &str) -> Result<ProjectorStatus> {
        match self.send_request(name, ProjectorRequest::GetStatus)? {
            ProjectorResponseOk::Status(status) => Ok(status),
            ProjectorResponseOk::Ok => {
                anyhow::bail!("Unexpected Ok response for GetStatus")
            }
        }
    }

    pub fn connect(&self, name: &str, port_path: &str) -> Result<()> {
        self.send_request(name, ProjectorRequest::Connect(port_path.to_string()))?;
        Ok(())
    }

    pub fn disconnect(&self, name: &str) -> Result<()> {
        self.send_request(name, ProjectorRequest::Disconnect)?;
        Ok(())
    }

    pub fn set_power(&self, name: &str, on: bool) -> Result<()> {
        self.send_request(name, ProjectorRequest::SetPower(on))?;
        Ok(())
    }

    pub fn set_av_mute(&self, name: &str, on: bool) -> Result<()> {
        self.send_request(name, ProjectorRequest::SetAvMute(on))?;
        Ok(())
    }

    pub fn set_eco_mode(&self, name: &str, eco: bool) -> Result<()> {
        self.send_request(name, ProjectorRequest::SetEcoMode(eco))?;
        Ok(())
    }

    fn send_request(&self, name: &str, request: ProjectorRequest) -> Result<ProjectorResponseOk> {
        let serialized = rmp_serde::to_vec(&request)?;
        let response_bytes = self.controller.send(name, &serialized)?;
        let response: ProjectorResponse = rmp_serde::from_slice(&response_bytes)?;
        response.map_err(|e| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_registers() {
        let stop =
            zero_configure::bare::register_service(SERVICE_NAME, 0).expect("should register");
        stop();
    }
}
