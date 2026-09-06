//! The show frame stream.
//!
//! One console publishes a frame of show state and every render client
//! attached to it receives that frame, so all of them draw the same instant of
//! the same show.

use anyhow::Result;
use log::{error, info};
use std::net::TcpListener;

use tunnels_model::show_frame::{
    FrameCodecError, FrameDecoder, FrameEncoder, ShowFrame, ShowFrameRef,
};

/// The port a show frame stream runs on.
const PORT: u16 = 6000;

/// The longest message this service accepts.
///
/// A length prefix arrives before the bytes it describes, so a reader that
/// trusted one would size a buffer to whatever a corrupted prefix — or one
/// sent by something that is not a publisher at all — happened to claim, up to
/// four gigabytes. The bound is what a reader checks the prefix against, and
/// nothing more.
///
/// It is deliberately far above any frame a show can produce, because the two
/// failures it sits between are not comparable. A prefix refused for being
/// absurd costs a reconnection. A frame refused for being long stops the show
/// with no symptom anyone watching it could act on, and stays broken.
const MAX_MESSAGE_LEN: usize = 128 * 1024 * 1024;

/// Puts show frames in front of every connected client.
///
/// Sending a frame encodes it and hands the bytes to each client's own sender
/// thread, so the cost of a send is the cost of the encoding and nothing a
/// client does. The encoding and the framing both write into buffers the
/// publisher keeps, so a steady stream of frames costs no allocation of its
/// own. Dropping the publisher releases the port and stops every thread it
/// started.
pub struct FramePublisher {
    publisher: minusmq::pub_sub::Publisher,
    encoder: FrameEncoder,
}

impl FramePublisher {
    /// Bind the frame port and begin accepting clients.
    pub fn new() -> Result<Self> {
        let listener = TcpListener::bind(format!("0.0.0.0:{PORT}"))?;
        let publisher = minusmq::pub_sub::Publisher::new(listener)?;
        info!("Frame server started.");
        Ok(Self {
            publisher,
            encoder: FrameEncoder::default(),
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

impl Drop for FramePublisher {
    fn drop(&mut self) {
        info!("Frame server shutting down.");
    }
}

/// Takes show frames off one console's stream of them.
///
/// A frame arrives in the buffer the frame before it arrived in and expands
/// into the scratch the frame before it used, so a steady stream of frames
/// costs the allocation of the frame itself and little else. The connection is
/// made on the first receive and remade whenever it is lost, so a subscriber
/// outlives the console it is attached to.
pub struct FrameSubscriber {
    subscriber: minusmq::pub_sub::Subscriber,
    decoder: FrameDecoder,
}

impl FrameSubscriber {
    /// Subscribe to the frames a console publishes.
    pub fn new(host: &str) -> Self {
        Self {
            subscriber: minusmq::pub_sub::Subscriber::new(host, PORT, MAX_MESSAGE_LEN),
            decoder: FrameDecoder::default(),
        }
    }

    /// Block until the next frame arrives, and recover it.
    ///
    /// Every way the bytes can be wrong is an error rather than a panic: each
    /// frame stands on its own, so bytes that do not describe one cost the
    /// frame they arrived in and nothing else.
    pub fn recv(&mut self) -> Result<ShowFrame, FrameCodecError> {
        self.decoder.decode(self.subscriber.recv())
    }
}
