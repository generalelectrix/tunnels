use std::marker::PhantomData;

use anyhow::{bail, Result};
use rmp_serde::Serializer;
use serde::{de::DeserializeOwned, Serialize};
use zmq::{Context, Socket, DONTWAIT};

use crate::{
    bare::{register_service, Browser, StopFn},
    msgpack::{Receive, ReceiveResult},
};

/// Advertise a DNS-SD pub/sub service, sending a stream of T using msgpack.
/// The service will be advertised until dropped.
pub struct PublisherService<T: Serialize> {
    stop: Option<StopFn>,
    socket: Socket,
    send_buf: Vec<u8>,
    _msg_type: PhantomData<T>,
}

impl<T: Serialize> PublisherService<T> {
    pub fn new(ctx: &Context, name: &str, port: u16) -> Result<Self> {
        let stop = register_service(name, port)?;
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

    pub fn send(&mut self, val: &T) -> Result<()> {
        self.send_buf.clear();
        val.serialize(&mut Serializer::new(&mut self.send_buf))?;
        self.socket.send(&self.send_buf, 0)?;
        Ok(())
    }
}

impl<T: Serialize> Drop for PublisherService<T> {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop()
        }
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
    /// Optionally filter to the provided topic.
    pub fn subscribe(&self, name: &str, topic: Option<&[u8]>) -> Result<Receiver<T>> {
        self.browser
            .use_service(name, move |cfg| {
                Receiver::new(&self.ctx, &cfg.hostname, cfg.port, topic)
            })
            .unwrap_or_else(|| bail!("no instance of service {} found", self.browser.name()))
    }
}

/// A strongly-typed 0mq SUB socket that expects messages to be encoded using msgpack.
pub struct Receiver<T: DeserializeOwned> {
    socket: Socket,
    has_topic: bool,
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> Receiver<T> {
    /// Create a new 0mq SUB connected to the provided socket addr.
    /// Expect a multipart message if a topic is provided.
    pub fn new(ctx: &Context, host: &str, port: u16, topic: Option<&[u8]>) -> Result<Self> {
        let socket = ctx.socket(zmq::SUB)?;
        let addr = format!("tcp://{}:{}", host, port);
        socket.connect(&addr)?;
        socket.set_subscribe(topic.unwrap_or(&[]))?;

        Ok(Self {
            socket,
            has_topic: topic.is_some(),
            _msg_type: PhantomData,
        })
    }

    pub fn receive_msg(&mut self, block: bool) -> ReceiveResult<Option<T>> {
        self.receive(block)
    }
}

impl<T: DeserializeOwned> Receive for Receiver<T> {
    fn receive_buffer(&mut self, block: bool) -> ReceiveResult<Option<Vec<u8>>> {
        let flag = if block { 0 } else { DONTWAIT };

        if self.has_topic {
            // The frame messages are two parts; the first part is the topic filter.
            // Discard the topic filter, leaving just the msgpacked data as the
            // second part of the message.
            match self.socket.recv_multipart(flag) {
                Ok(mut parts) => {
                    if parts.len() != 2 {
                        bail!("buffer receieve error, expected a 2-part message but got {} parts: {:?}", parts.len(), parts);
                    }
                    Ok(parts.pop())
                }
                Err(zmq::Error::EAGAIN) => {
                    // No message was available.
                    Ok(None)
                }
                Err(other_err) => bail!("buffer receieve error: {other_err}"),
            }
        } else {
            match self.socket.recv_bytes(flag) {
                Ok(msg) => Ok(Some(msg)),
                Err(zmq::Error::EAGAIN) => Ok(None),
                Err(other_err) => bail!("buffer receieve error: {other_err}"),
            }
        }
    }
}
