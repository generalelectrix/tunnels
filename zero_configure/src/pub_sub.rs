use std::{error::Error, marker::PhantomData};

use rmp_serde::Serializer;
use serde::{Deserialize, Serialize};
use zmq::{Context, Socket};

use crate::bare::{browse_forever, register_service, StopFn};

/// Advertise a DNS-SD pub/sub service, sending a stream of T using msgpack.
/// The service will be advertised until dropped.
pub struct PublisherService<T: Serialize> {
    stop: Option<StopFn>,
    socket: Socket,
    send_buf: Vec<u8>,
    _msg_type: PhantomData<T>,
}

impl<T: Serialize> PublisherService<T> {
    pub fn new(ctx: Context, name: &str, port: u16) -> Result<Self, Box<dyn Error>> {
        let stop = register_service(&name, port)?;
        let socket = ctx.socket(zmq::PUB)?;
        let addr = format!("tcp://*:{}", port);
        socket.bind(&addr)?;
        Ok(Self {
            stop: Some(stop),
            socket,
            send_buf: Vec::new(),
            _msg_type: PhantomData,
        })
    }

    pub fn send(&mut self, val: &T) -> Result<(), Box<dyn Error>> {
        self.send_buf.clear();
        val.serialize(&mut Serializer::new(&mut self.send_buf))?;
        self.socket.send(&self.send_buf, 0)?;
        Ok(())
    }
}

impl<T: Serialize> Drop for PublisherService<T> {
    fn drop(&mut self) {
        self.stop.take().map(|stop| stop());
    }
}

pub struct SubscriberService<T> {
    stop: Option<StopFn>,
    socket: Socket,
    send_buf: Vec<u8>,
    _msg_type: PhantomData<T>,
}
