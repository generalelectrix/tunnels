use std::{
    error::Error,
    sync::mpsc::{channel, Receiver, Sender, TryRecvError},
};

use log::{error, info, warn};
use rmp_serde::Serializer;
use serde::Serialize;
use std::thread;
use tunnels_lib::{Snapshot, Timestamp};
use zmq::{Context, Socket};

use crate::{clock::ClockBank, mixer::Mixer};

const PORT: u16 = 6000;

/// Renders the show state and sends it to all connected clients.
/// Returns a channel for sending frames to be rendered.
/// The service runs until the channel is dropped.
pub fn start_render_service(ctx: &mut Context) -> Result<Sender<Frame>, Box<dyn Error>> {
    let socket = ctx.socket(zmq::PUB)?;
    let addr = format!("tcp://*:{}", PORT);
    socket.bind(&addr)?;

    let (send, mut recv) = channel();

    let mut send_buf = Vec::new();
    thread::Builder::new()
        .name("render".to_string())
        .spawn(move || loop {
            match get_frame(&mut recv) {
                None => {
                    info!("Render server shutting down.");
                    return;
                }
                Some((dropped_frames, frame)) => {
                    if dropped_frames > 0 {
                        warn!("Render server dropped {} frames.", dropped_frames);
                    }

                    let video_outs = frame.mixer.render(&frame.clocks);
                    for (video_chan, draw_commands) in video_outs.into_iter().enumerate() {
                        let snapshot = Snapshot {
                            frame_number: frame.number,
                            time: frame.timestamp,
                            layers: draw_commands,
                        };
                        send_snapshot(&mut send_buf, &socket, video_chan, snapshot);
                    }
                }
            }
        })?;
    info!("Render server started.");
    Ok(send)
}

/// Block until a frame is available.
/// Also optimistically check if there is already one or more frames backed up
/// behind the first frame.  If so, drain them all and return the last frame
/// received as well as the number of dropped frames.
/// If the receiver has disconnected, return None.
fn get_frame(recv: &mut Receiver<Frame>) -> Option<(u32, Frame)> {
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

/// Serialize the provided snapshot and send it to the specified video channel.
/// Error conditions are logged.
fn send_snapshot(
    mut send_buf: &mut Vec<u8>,
    socket: &Socket,
    video_channel: usize,
    snapshot: Snapshot,
) {
    let topic = [video_channel as u8; 1];
    send_buf.clear();

    if let Err(e) = snapshot.serialize(&mut Serializer::new(&mut send_buf)) {
        error!(
            "Snapshot serialization error for frame {} channel {}: {}.",
            snapshot.frame_number, video_channel, e,
        );
        return;
    }

    let messages: [&[u8]; 2] = [&topic, send_buf];
    if let Err(e) = socket.send_multipart(messages.iter(), 0) {
        error!(
            "Snapshot send error for frame {} channel {}: {}.",
            snapshot.frame_number, video_channel, e,
        );
    }
}

pub struct Frame {
    pub number: u64,
    pub timestamp: Timestamp,
    pub mixer: Mixer,
    pub clocks: ClockBank,
}
