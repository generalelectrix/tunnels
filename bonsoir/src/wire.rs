//! Wire protocol: msgpack-encoded packets for heartbeat, query, and goodbye.

use std::net::IpAddr;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Current protocol version. Receivers should log a warning and skip packets
/// with a different version.
pub const PROTOCOL_VERSION: u8 = 1;

/// The type of message in a bonsoir packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    /// Periodic announcement: "I'm here."
    Heartbeat,
    /// Browser asking all registrars to respond: "Who's there?"
    Query,
    /// Clean shutdown: "I'm leaving."
    Goodbye,
}

/// A bonsoir wire packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packet {
    pub version: u8,
    pub message_type: MessageType,
    pub service_type: String,
    pub instance_name: String,
    pub port: u16,
}

impl Packet {
    /// Build a Heartbeat packet for the given service.
    pub fn heartbeat(service_type: &str, instance_name: &str, port: u16) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Heartbeat,
            service_type: service_type.to_string(),
            instance_name: instance_name.to_string(),
            port,
        }
    }

    /// Build a Query packet for the given service type.
    pub fn query(service_type: &str) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Query,
            service_type: service_type.to_string(),
            instance_name: String::new(),
            port: 0,
        }
    }

    /// Build a Goodbye packet for the given service.
    pub fn goodbye(service_type: &str, instance_name: &str, port: u16) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Goodbye,
            service_type: service_type.to_string(),
            instance_name: instance_name.to_string(),
            port,
        }
    }

    /// Serialize this packet to bytes.
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(self)?)
    }

    /// Deserialize a packet from bytes.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        Ok(rmp_serde::from_slice(buf)?)
    }
}

/// A discovered service instance, combining packet data with the sender's address.
#[derive(Debug, Clone)]
pub struct ServiceInstance {
    pub service_type: String,
    pub instance_name: String,
    pub address: IpAddr,
    pub port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_round_trip() {
        let pkt = Packet::heartbeat("tunnelbootstrap", "Bore A", 15000);
        let bytes = pkt.encode().unwrap();
        let decoded = Packet::decode(&bytes).unwrap();
        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.message_type, MessageType::Heartbeat);
        assert_eq!(decoded.service_type, "tunnelbootstrap");
        assert_eq!(decoded.instance_name, "Bore A");
        assert_eq!(decoded.port, 15000);
    }

    #[test]
    fn query_round_trip() {
        let pkt = Packet::query("showclocks");
        let bytes = pkt.encode().unwrap();
        let decoded = Packet::decode(&bytes).unwrap();
        assert_eq!(decoded.message_type, MessageType::Query);
        assert_eq!(decoded.service_type, "showclocks");
    }

    #[test]
    fn goodbye_round_trip() {
        let pkt = Packet::goodbye("tunnelbootstrap", "Bore A", 15000);
        let bytes = pkt.encode().unwrap();
        let decoded = Packet::decode(&bytes).unwrap();
        assert_eq!(decoded.message_type, MessageType::Goodbye);
        assert_eq!(decoded.instance_name, "Bore A");
    }

    #[test]
    fn packet_is_small() {
        let pkt = Packet::heartbeat("tunnelbootstrap", "Bore A", 15000);
        let bytes = pkt.encode().unwrap();
        assert!(bytes.len() < 100, "packet should be small, got {} bytes", bytes.len());
    }
}
