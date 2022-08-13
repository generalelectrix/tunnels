use derive_more::Display;
use log::error;
use rosc::{OscMessage, OscPacket, OscType};
use simple_error::bail;
use std::error::Error;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;
use tunnels_lib::color::Rgb;

use crate::control::ControlEvent;
use crate::master_ui::EmitStateChange;
use crate::palette::{ControlMessage as PaletteControlMessage, StateChange as PaletteStateChange};
use crate::show::{ControlMessage, StateChange};

/// The OSC device types that tunnels can work with.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display)]
pub enum Device {
    PaletteController,
}

/// Wrapper struct for the data needed to describe a device to connect to.
#[derive(Clone, Debug, Copy)]
pub struct DeviceSpec {
    pub device: Device,
    pub addr: SocketAddr,
}

pub struct Dispatcher {
    _inputs: Vec<Input>,
}

impl Dispatcher {
    pub fn new(
        osc_devices: Vec<DeviceSpec>,
        send: Sender<ControlEvent>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut inputs = Vec::new();
        for osc_device in osc_devices {
            inputs.push(Input::new(osc_device, send.clone())?);
        }
        Ok(Self { _inputs: inputs })
    }

    pub fn map_event_to_show_control(
        &self,
        device: Device,
        event: OscMessage,
    ) -> Result<ControlMessage, Box<dyn Error>> {
        match event.addr.as_str() {
            "/palette" => handle_palette(event.args),
            unknown => {
                bail!(
                    "Unknown OSC command from device {} with address {}: {:?}",
                    device,
                    unknown,
                    event.args
                )
            }
        }
    }
}

/// Process a vector of OSC types that are expected to represent a color palette.
/// Ignore the message and log
fn handle_palette(args: Vec<OscType>) -> Result<ControlMessage, Box<dyn Error>> {
    // Scan the input vector, extracting colors and converting to HSV.
    let mut colors = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if let OscType::Color(color) = arg {
            colors.push(Rgb::from_8bit(color.red, color.green, color.blue).as_hsv());
        } else {
            bail!("Unexpected OSC type in palette at index {}: {:?}.", i, arg);
        }
    }
    Ok(ControlMessage::ColorPalette(PaletteControlMessage::Set(
        PaletteStateChange::Contents(colors),
    )))
}

impl EmitStateChange for Dispatcher {
    /// Map application state changes into OSC update midi messages.
    fn emit(&mut self, _: StateChange) {
        // For the moment there's no talkback over OSC.
    }
}

/// Input is a OSC input, forwarding OSC messages to the provided sender.
/// Spawns a new thread to handle listening for messages.
struct Input(DeviceSpec);

impl Input {
    pub fn new(spec: DeviceSpec, send: Sender<ControlEvent>) -> Result<Self, Box<dyn Error>> {
        let socket = UdpSocket::bind(spec.addr)?;

        let mut buf = [0u8; rosc::decoder::MTU];

        let mut recv = move || -> Result<OscPacket, Box<dyn Error>> {
            let size = socket.recv(&mut buf)?;
            let (_, packet) = rosc::decoder::decode_udp(&buf[..size])?;
            Ok(packet)
        };

        thread::spawn(move || loop {
            match recv() {
                Ok(packet) => {
                    forward_packet(packet, spec.device, &send);
                }
                Err(e) => {
                    error!("Error receiving from OSC device {}: {}", spec.device, e);
                }
            }
        });
        Ok(Self(spec))
    }
}

/// Recursively unpack OSC packets and send all the inner messages as control events.
fn forward_packet(packet: OscPacket, device: Device, send: &Sender<ControlEvent>) {
    match packet {
        OscPacket::Message(m) => {
            send.send(ControlEvent::Osc((device, m))).unwrap();
        }
        OscPacket::Bundle(msgs) => {
            for subpacket in msgs.content {
                forward_packet(subpacket, device, send);
            }
        }
    }
}
