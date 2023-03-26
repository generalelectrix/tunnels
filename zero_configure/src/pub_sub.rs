use std::{error::Error, marker::PhantomData};

use rmp_serde::Serializer;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zmq::{Context, Socket};

use crate::bare::{browse_forever, register_service, Browser, StopFn};

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

struct SubConfig {
    hostname: String,
    port: u16,
}

pub struct SubscriberService<T: DeserializeOwned> {
    browser: Browser<SubConfig>,
    ctx: Context,
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> SubscriberService<T> {
    /// Browse for publishers of the named service.
    /// Connect SUB sockets upon request.
    pub fn new(ctx: Context, name: String) -> Self {
        Self {
            browser: Browser::new(name, |service| {
                Ok(SubConfig {
                    hostname: service.host_target.clone(),
                    port: service.port,
                })
            }),
            ctx,
            _msg_type: PhantomData,
        }
    }

    /// List the services currently available.
    pub fn list(&self) -> Vec<String> {
        self.browser.list()
    }

    /// Connect a SUB socket to a service.
    pub fn subscribe(name: &str) -> Result<Receiver<T>, Box<dyn Error>> {
        unimplemented!()
    }
}

/// A strongly-typed 0mq SUB socket.
pub struct Receiver<T: DeserializeOwned> {
    socket: Socket,
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> Receiver<T> {
    fn new(ctx: &Context, cfg: SubConfig) -> Result<Self, Box<dyn Error>> {
        unimplemented!()
    }
}
