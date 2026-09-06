//! The network service a show is made of: the show frame stream.
//!
//! One console publishes a frame of show state and every render client
//! attached to it receives that frame, so all of them draw the same instant of
//! the same show.
//!
//! The service owns its protocol at both ends: the port it runs on, the
//! payload it carries, the ceiling on a message of it, and the halves that
//! publish and consume it. Nothing about the protocol is written at a call
//! site, so the two ends of it cannot drift apart.

use anyhow::Result;
use log::{error, info};
use minusmq::pub_sub::{Compression, Config};
use std::net::TcpListener;

pub use minusmq::pub_sub::SubscriberStop;

use tunnels_model::show_frame::{FrameCodecError, FrameEncoder, ShowFrame, ShowFrameRef};

/// The port a show frame stream runs on.
const PORT: u16 = 6000;

/// The largest payload a frame of this service is allowed to expand into.
///
/// A compressed frame is a fraction of the size it expands to, so the length
/// a message arrives at bounds nothing about the memory expanding it asks
/// for. A frame is a few kilobytes. The ceiling is here so that a corrupted
/// payload asks for a rejected expansion rather than a multi-gigabyte
/// allocation.
const MAX_DECOMPRESSED_LEN: usize = 8 * 1024 * 1024;

/// How the show frame stream is carried.
///
/// A frame is a few kilobytes of model that compresses well, and there is one
/// of them every four milliseconds for every client, so the stream carries
/// them compressed. The message ceiling is the transport's own, which sits
/// deliberately far above any frame a show can produce: a prefix refused for
/// being absurd costs a reconnection, while a frame refused for being long
/// stops the show with no symptom anyone watching it could act on.
fn config() -> Config {
    Config {
        compression: Compression::Lz4 {
            max_decompressed_len: MAX_DECOMPRESSED_LEN,
        },
        ..Default::default()
    }
}

/// Puts show frames in front of every connected client.
///
/// Sending a frame encodes it and hands the bytes to each client's own sender
/// thread, so the cost of a send is the cost of the encoding and nothing a
/// client does. The encoding, the compression and the framing all write into
/// buffers the publisher keeps, so a steady stream of frames costs no
/// allocation of its own. Dropping the publisher releases the port and stops
/// every thread it started.
pub struct FramePublisher {
    publisher: minusmq::pub_sub::Publisher,
    encoder: FrameEncoder,
}

impl FramePublisher {
    /// Bind the frame port and begin accepting clients.
    pub fn new() -> Result<Self> {
        let publisher = Self::on(TcpListener::bind(format!("0.0.0.0:{PORT}"))?)?;
        info!("Frame server started.");
        Ok(publisher)
    }

    /// Publish frames to the clients an already-bound listener accepts.
    fn on(listener: TcpListener) -> Result<Self> {
        Ok(Self {
            publisher: minusmq::pub_sub::Publisher::new(listener, config())?,
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
            Err(e) => error!("Frame serialization error: {e}."),
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
}

impl FrameSubscriber {
    /// Subscribe to the frames a console publishes.
    pub fn new(host: &str) -> Self {
        Self::at(host, PORT)
    }

    /// Subscribe to the frames published on one host and port.
    fn at(host: &str, port: u16) -> Self {
        Self {
            subscriber: minusmq::pub_sub::Subscriber::new(host, port, config()),
        }
    }

    /// A handle that stops this subscription from another thread.
    pub fn stop_handle(&self) -> SubscriberStop {
        self.subscriber.stop_handle()
    }

    /// Block until the next frame arrives, and recover it. Yields nothing once
    /// the subscription has been stopped.
    ///
    /// Every way the bytes can be wrong is an error rather than a panic: each
    /// frame stands on its own, so bytes that do not describe one cost the
    /// frame they arrived in and nothing else.
    pub fn recv(&mut self) -> Option<Result<ShowFrame, FrameCodecError>> {
        Some(ShowFrame::decode(self.subscriber.recv()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpStream;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::{Duration, Instant};
    use tunnels_model::show_frame::fixture::{self, NamedFrame};

    /// How long a test waits on the stream before taking a frame not to be
    /// coming.
    const LIMIT: Duration = Duration::from_secs(10);

    /// How long a subscription gets to be taken up by the accept thread.
    ///
    /// A subscription is taken up there rather than by the connect that starts
    /// it, so the moment a frame first has somewhere to go is not one a test
    /// can name.
    const SUBSCRIBE_DELAY: Duration = Duration::from_millis(300);

    /// The same frame, named by reference rather than owned.
    fn borrow(frame: &ShowFrame) -> ShowFrameRef<'_> {
        ShowFrameRef {
            mixer: &frame.mixer,
            clocks: frame.clocks.clone(),
            palette: &frame.palette,
            positions: &frame.positions,
            audio_envelope: frame.audio_envelope,
        }
    }

    /// A publisher of frames on a port of the operating system's choosing.
    fn test_publisher() -> (FramePublisher, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        (FramePublisher::on(listener).unwrap(), port)
    }

    /// The bytes one frame goes out as, stated without reference to the code
    /// that produces them: the length of the payload big-endian, then the
    /// frame in postcard as an LZ4 block behind the length it expands to.
    fn wire_format(frame: &ShowFrameRef) -> Vec<u8> {
        let payload = lz4_flex::compress_prepend_size(&postcard::to_allocvec(frame).unwrap());
        let mut wire = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        wire.extend_from_slice(&payload);
        wire
    }

    /// A published frame reaches the socket as the bytes the wire format
    /// defines.
    ///
    /// The bytes are a contract between applications rather than an internal
    /// detail: every render client reads them, and reads them from a binary
    /// built at another time. They are held against an independent statement
    /// of the format, so that a change to how the frame is encoded, compressed
    /// or framed fails here instead of quietly redefining what a client has to
    /// read.
    #[test]
    fn a_published_frame_is_byte_for_byte_the_wire_format() {
        let (mut publisher, port) = test_publisher();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_read_timeout(Some(LIMIT)).unwrap();
        thread::sleep(SUBSCRIBE_DELAY);

        for NamedFrame { name, frame } in fixture::all() {
            let expected = wire_format(&borrow(&frame));
            publisher.send(&borrow(&frame));
            let mut actual = vec![0u8; expected.len()];
            client
                .read_exact(&mut actual)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                actual == expected,
                "{name}: went out as {} bytes against the {} the wire format defines",
                actual.len(),
                expected.len()
            );
        }
    }

    /// A frame published on the stream is the same frame at the other end of
    /// it.
    #[test]
    fn a_frame_survives_the_stream() {
        let (mut publisher, port) = test_publisher();
        let subscriber = FrameSubscriber::at("127.0.0.1", port);
        let stop = subscriber.stop_handle();
        let (received, receipts) = channel();
        thread::spawn({
            let mut subscriber = subscriber;
            move || {
                while let Some(frame) = subscriber.recv() {
                    if received.send(frame.map_err(|e| e.to_string())).is_err() {
                        return;
                    }
                }
            }
        });
        thread::sleep(SUBSCRIBE_DELAY);

        // The frame that spends every animation slot and reads every bank, so
        // that the stream is held to the widest model a show can produce.
        let published = fixture::max_variation_frame();
        // Republishing is free and a frame is idempotent, so the wait for a
        // subscription to be taken up does not have to be exact.
        let deadline = Instant::now() + LIMIT;
        let decoded = loop {
            publisher.send(&borrow(&published));
            if let Ok(received) = receipts.recv_timeout(Duration::from_millis(50)) {
                break received.unwrap();
            }
            assert!(Instant::now() < deadline, "no frame arrived in {LIMIT:?}");
        };
        stop.stop();

        assert_eq!(format!("{decoded:#?}"), format!("{published:#?}"));
    }
}
