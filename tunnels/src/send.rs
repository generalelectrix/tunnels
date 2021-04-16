use std::sync::mpsc::Receiver;

use zmq::Socket;

use crate::{clock::ClockBank, mixer::Mixer};

/// Renders the show state and sends it to all connected clients.
pub struct RenderSender {
    socket: Socket,
    buffer: Receiver<Frame>,
}

pub struct Frame {
    mixer: Mixer,
    clocks: ClockBank,
}
