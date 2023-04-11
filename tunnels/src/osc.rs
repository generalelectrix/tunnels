use anyhow::bail;
use anyhow::Result;
use derive_more::Display;
use log::{debug, error, warn};
use rosc::{OscMessage, OscPacket, OscType};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::thread;
use tunnels_lib::color::Rgb;
use tunnels_lib::number::UnipolarFloat;

use crate::control::ControlEvent;
use crate::master_ui::EmitStateChange;
use crate::palette::{ControlMessage as PaletteControlMessage, StateChange as PaletteStateChange};
use crate::position_bank::Position;
use crate::show::{ControlMessage, StateChange};

/// The OSC device types that tunnels can work with.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display)]
pub enum Device {
    PaletteController,
    PositionController,
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
    pub fn new(osc_devices: Vec<DeviceSpec>, send: Sender<ControlEvent>) -> Result<Self> {
        let mut inputs = Vec::new();
        for osc_device in osc_devices {
            inputs.push(Input::new(osc_device, send.clone())?);
        }
        Ok(Self { _inputs: inputs })
    }

    /// Map the provided OSC event to a show control message.
    /// Return None if the event does not map to a known control.
    pub fn map_event_to_show_control(
        &self,
        device: Device,
        event: OscMessage,
    ) -> Result<Option<ControlMessage>> {
        match event.addr.as_str() {
            "/palette" => handle_palette(event.args).map(Some),
            "/position" => handle_position(event.args).map(Some),
            unknown => {
                debug!(
                    "Unknown OSC command from device {} with address {}: {:?}",
                    device, unknown, event.args
                );
                Ok(None)
            }
        }
    }
}

/// Process a vector of OSC types that are expected to represent a color palette.
fn handle_palette(args: Vec<OscType>) -> Result<ControlMessage> {
    // Scan the input vector, extracting colors and converting to HSV.
    let colors = handle_osc_vec_chunks(args, 3, |chunk| {
        Ok(Rgb {
            red: get_osc_unipolar(&chunk[0])?,
            green: get_osc_unipolar(&chunk[1])?,
            blue: get_osc_unipolar(&chunk[2])?,
        }
        .as_hsv())
    })?;
    Ok(ControlMessage::ColorPalette(PaletteControlMessage::Set(
        PaletteStateChange::Contents(colors),
    )))
}

/// Process a vector of OSC types that are expected to represent a X/Y position.
fn handle_position(args: Vec<OscType>) -> Result<ControlMessage> {
    Ok(ControlMessage::Position(handle_osc_vec_chunks(
        args,
        2,
        |chunk| {
            Ok(Position {
                x: get_osc_float(&chunk[0])?,
                y: get_osc_float(&chunk[1])?,
            })
        },
    )?))
}

/// Process a vector of OSC types as a series of chunks.
fn handle_osc_vec_chunks<T>(
    args: Vec<OscType>,
    chunk_size: usize,
    chunk_proc: impl FnMut(&[OscType]) -> Result<T>,
) -> Result<Vec<T>> {
    args.chunks(chunk_size).filter(|chunk| {
        if chunk.len() < chunk_size {
            warn!("OSC message had a trailing chunk with less than {chunk_size} components: {chunk:?}");
            false
        } else {
            true
        }
    }).map(chunk_proc).collect()
}

fn get_osc_unipolar(v: &OscType) -> Result<UnipolarFloat> {
    get_osc_float(v).map(UnipolarFloat::new)
}

fn get_osc_float(v: &OscType) -> Result<f64> {
    match v {
        OscType::Float(v) => Ok(*v as f64),
        OscType::Double(v) => Ok(*v),
        other => {
            bail!(
                "Unexpected OSC type; expected a float or double, got {:?}.",
                other
            )
        }
    }
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
    pub fn new(spec: DeviceSpec, send: Sender<ControlEvent>) -> Result<Self> {
        let socket = UdpSocket::bind(spec.addr)?;

        let mut buf = [0u8; rosc::decoder::MTU];

        let mut recv = move || -> Result<OscPacket> {
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
