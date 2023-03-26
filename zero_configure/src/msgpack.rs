//! Helper trait for working with msgpack.

use rmp_serde::decode::Error as DecodeError;
use rmp_serde::Deserializer;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::io::Cursor;

pub type ReceiveResult<T> = Result<T, DecodeError>;

pub trait Receive {
    /// Return the raw message buffer if one was available.
    /// The implementation should block until a message is available if block
    /// is true.
    fn receive_buffer(&mut self, block: bool) -> Option<Vec<u8>>;

    /// Deserialize a received message.
    fn deserialize_msg<T: DeserializeOwned>(&self, msg: Vec<u8>) -> ReceiveResult<T> {
        let cur = Cursor::new(&msg[..]);
        let mut de = Deserializer::new(cur);
        Deserialize::deserialize(&mut de)
    }

    /// Receive a single message.
    fn receive<T: DeserializeOwned>(&mut self, block: bool) -> Option<ReceiveResult<T>> {
        if let Some(buf) = self.receive_buffer(block) {
            Some(self.deserialize_msg(buf))
        } else {
            None
        }
    }
}
