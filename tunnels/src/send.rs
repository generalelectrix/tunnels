use anyhow::Result;
use log::{error, info, warn};
use std::net::TcpListener;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;

use crate::show_frame::ShowFrame;

const PORT: u16 = 6000;

/// The publish-subscribe channel show frames are published on.
///
/// A frame describes the whole show, so every client subscribes to the same
/// channel, receives the same bytes, and selects its own video channel out of
/// them when it renders.
const FRAME_CHANNEL: u8 = 0;

/// Publishes show frames to all connected clients.
/// Returns a channel for sending frames to be published.
/// The service runs until the channel is dropped.
pub fn start_frame_service() -> Result<Sender<ShowFrame>> {
    let listener = TcpListener::bind(format!("0.0.0.0:{PORT}"))?;
    let publisher = minusmq::pub_sub::Publisher::new(listener)?;

    let (send, mut recv) = channel();

    thread::Builder::new()
        .name("frame_publisher".to_string())
        .spawn(move || {
            loop {
                match get_frame(&mut recv) {
                    None => {
                        info!("Frame server shutting down.");
                        return;
                    }
                    Some((dropped_frames, frame)) => {
                        if dropped_frames > 0 {
                            warn!("Frame server dropped {dropped_frames} frames.");
                        }
                        send_frame(&publisher, &frame);
                    }
                }
            }
        })?;
    info!("Frame server started.");
    Ok(send)
}

/// Block until a frame is available.
/// Also optimistically check if there is already one or more frames backed up
/// behind the first frame.  If so, drain them all and return the last frame
/// received as well as the number of dropped frames.
/// If the receiver has disconnected, return None.
fn get_frame(recv: &mut Receiver<ShowFrame>) -> Option<(u32, ShowFrame)> {
    let mut dropped_frames = 0;
    // Wait for a frame.
    let mut frame = match recv.recv() {
        Ok(frame) => frame,
        Err(_) => return None,
    };
    loop {
        match recv.try_recv() {
            Ok(newer_frame) => {
                dropped_frames += 1;
                frame = newer_frame;
            }
            Err(TryRecvError::Empty) => {
                return Some((dropped_frames, frame));
            }
            Err(TryRecvError::Disconnected) => {
                return None;
            }
        }
    }
}

/// Encode the provided frame and send it to every subscribed client.
/// Error conditions are logged.
fn send_frame(publisher: &minusmq::pub_sub::Publisher, frame: &ShowFrame) {
    match frame.encode() {
        Ok(bytes) => publisher.send(FRAME_CHANNEL, &bytes),
        Err(e) => error!(
            "Frame serialization error for frame {}: {e}.",
            frame.frame_number
        ),
    }
}
