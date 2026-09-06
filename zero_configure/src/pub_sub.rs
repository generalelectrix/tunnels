//! DNS-SD-integrated publish-subscribe using minusmq TCP pub/sub.

use std::marker::PhantomData;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};

use anyhow::{Result, bail};
use rmp_serde::Serializer;
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    bare::{Browser, StopFn, register_service},
    msgpack::{Receive, ReceiveResult},
};

/// Advertise a DNS-SD pub/sub service, sending a stream of T using msgpack.
/// The service will be advertised until dropped.
pub struct PublisherService<T: Serialize> {
    stop: Option<StopFn>,
    publisher: minusmq::pub_sub::Publisher,
    send_buf: Vec<u8>,
    _msg_type: PhantomData<T>,
}

impl<T: Serialize> PublisherService<T> {
    pub fn new(name: &str, port: u16) -> Result<Self> {
        let stop = register_service(name, port)?;
        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
        let publisher = minusmq::pub_sub::Publisher::new(listener)?;
        Ok(Self {
            stop: Some(stop),
            publisher,
            send_buf: Vec::new(),
            _msg_type: PhantomData,
        })
    }

    pub fn send(&mut self, val: &T) -> Result<()> {
        self.send_buf.clear();
        val.serialize(&mut Serializer::new(&mut self.send_buf))?;
        self.publisher.send(&self.send_buf);
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
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> SubscriberService<T> {
    /// Browse for publishers of the named service.
    /// Connect subscribers upon request.
    pub fn new(name: String) -> Self {
        Self {
            browser: Browser::new(name, |service| {
                Ok(SubConfig {
                    hostname: service.hostname.clone(),
                    port: service.port,
                })
            }),
            _msg_type: PhantomData,
        }
    }

    /// List the services currently available.
    pub fn list(&self) -> Vec<String> {
        self.browser.list()
    }

    /// Connect a subscriber to the named service.
    pub fn subscribe(&self, name: &str) -> Result<Receiver<T>> {
        self.browser
            .use_service(name, move |cfg| {
                // Resolve hostname to IP at subscribe time.
                let addr: SocketAddr = (&*cfg.hostname, cfg.port)
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Could not resolve {}:{}", cfg.hostname, cfg.port)
                    })?;
                Ok(Receiver::new(&addr.ip().to_string(), addr.port()))
            })
            .unwrap_or_else(|| bail!("no instance of service {} found", self.browser.name()))
    }
}

/// A strongly-typed TCP subscriber that expects messages to be encoded using msgpack.
pub struct Receiver<T: DeserializeOwned> {
    subscriber: minusmq::pub_sub::Subscriber,
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> Receiver<T> {
    /// Create a new subscriber connected to the provided host:port.
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            subscriber: minusmq::pub_sub::Subscriber::new(host, port),
            _msg_type: PhantomData,
        }
    }

    pub fn receive_msg(&mut self, block: bool) -> ReceiveResult<Option<T>> {
        self.receive(block)
    }
}

impl<T: DeserializeOwned> Receive for Receiver<T> {
    fn receive_buffer(&mut self, _block: bool) -> ReceiveResult<Option<Vec<u8>>> {
        // minusmq subscriber always blocks. The `block` parameter is preserved
        // for API compatibility but is effectively always true.
        Ok(Some(self.subscriber.recv().to_vec()))
    }
}
