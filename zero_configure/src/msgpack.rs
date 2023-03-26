//! Helper trait for working with msgpack.

use rmp_serde::Deserializer;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::error::Error;
use std::io::Cursor;

pub type ReceiveResult<T> = Result<T, Box<dyn Error>>;

pub trait Receive {
    /// Return the raw message buffer if one was available.
    /// The implementation should block until a message is available if block
    /// is true.
    fn receive_buffer(&mut self, block: bool) -> ReceiveResult<Option<Vec<u8>>>;

    /// Deserialize a received message.
    fn deserialize_msg<T: DeserializeOwned>(&self, msg: Vec<u8>) -> ReceiveResult<T> {
        let cur = Cursor::new(&msg[..]);
        let mut de = Deserializer::new(cur);
        let typed_msg = Deserialize::deserialize(&mut de)?;
        Ok(typed_msg)
    }

    /// Receive a single message.
    fn receive<T: DeserializeOwned>(&mut self, block: bool) -> ReceiveResult<Option<T>> {
        self.receive_buffer(block)?
            .map(|buf| self.deserialize_msg(buf))
            .transpose()
    }
}
