//! 0mq communication and deserialization.

use log::error;
use serde::de::DeserializeOwned;

use std::error::Error;

use std::sync::mpsc::{channel, Receiver};
use std::thread;
use zero_configure::pub_sub::Receiver as SubReceiver;

/// Spawn a thread and pass SUB messages onto a channel.
/// This will run until the returned channel is dropped.
pub fn receive_async<T>(mut receiver: SubReceiver<T>) -> Result<Receiver<T>, Box<dyn Error>>
where
    T: DeserializeOwned + Send + 'static,
{
    let (tx, rx) = channel::<T>();
    thread::Builder::new()
        .name("subscribe_receiver".to_string())
        .spawn(move || {
            loop {
                // blocking receive
                match receiver.receive_msg(true) {
                    Ok(Some(msg)) => {
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
