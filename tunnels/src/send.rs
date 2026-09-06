use anyhow::Result;
use log::{error, info};
use std::net::TcpListener;

use tunnels_model::show_frame::{FrameEncoder, ShowFrameRef};

const PORT: u16 = 6000;

/// Puts show frames in front of every connected client.
///
/// Sending a frame encodes it and hands the bytes to each client's own sender
/// thread, so the cost of a send is the cost of the encoding and nothing a
/// client does. The encoding and the framing both write into buffers the
/// service keeps, so a steady stream of frames costs no allocation of its own.
/// Dropping the service releases the port and stops every thread it started.
pub struct FrameService {
    publisher: minusmq::pub_sub::Publisher,
    encoder: FrameEncoder,
}

impl FrameService {
    /// Bind the frame port and begin accepting clients.
    pub fn new() -> Result<Self> {
        let listener = TcpListener::bind(format!("0.0.0.0:{PORT}"))?;
        let publisher = minusmq::pub_sub::Publisher::new(listener)?;
        info!("Frame server started.");
        Ok(Self {
            publisher,
            encoder: FrameEncoder::new(),
        })
    }

    /// Encode a frame and give it to every subscribed client.
    ///
    /// A frame that cannot be serialized is reported and skipped: one frame is
    /// worth less than the show it would take down.
    pub fn send(&mut self, frame: &ShowFrameRef) {
        match self.encoder.encode(frame) {
            Ok(bytes) => self.publisher.send(bytes),
            Err(e) => error!(
                "Frame serialization error for frame {}: {e}.",
                frame.frame_number
            ),
        }
    }
}

impl Drop for FrameService {
    fn drop(&mut self) {
        info!("Frame server shutting down.");
    }
}
