//! 0mq communication and deserialization.

use log::error;

use anyhow::Result;
use tunnels_lib::Snapshot;

use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use zero_configure::pub_sub::Receiver as SubReceiver;

use crate::timesync::Synchronizer;

/// Spawn a thread and pass SUB messages onto a channel.
/// This will run until the returned channel is dropped.
pub fn receive_async(
    mut receiver: SubReceiver<Snapshot>,
    timesync: Arc<Mutex<Synchronizer>>,
) -> Result<Receiver<Snapshot>> {
    let (tx, rx) = channel::<Snapshot>();
    thread::Builder::new()
        .name("subscribe_receiver".to_string())
        .spawn(move || {
            loop {
                // blocking receive
                match receiver.receive_msg(true) {
                    Ok(Some(msg)) => {
                        let current_time = timesync.lock().unwrap().now();
                        println!("received snapshot; delay: {}", current_time - msg.time);
                        // post message to queue
                        // if a send fails, the other side has hung up and we should quit
                        match tx.send(msg) {
                            Ok(_) => continue,
                            Err(_) => break,
                        }
                    }
                    Ok(None) => continue, // Odd case, given that we should have blocked.
                    Err(e) => error!("receive error: {e}"),
                }
            }
        })?;
    Ok(rx)
}
