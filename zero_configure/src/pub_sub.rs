//! DNS-SD-integrated publish-subscribe using minusmq TCP pub/sub.
//!
//! The stream is postcard: tagless, so the schema is the Rust type and does
//! not travel with the message.
//!
//! Both halves are published interface. A stream advertised here is consumed
//! by separately deployed applications as well as by this workspace, so the
//! subscribing half stands on the service being advertised rather than on
//! anything in this workspace receiving it.

use std::marker::PhantomData;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};

use anyhow::{Result, bail};
use serde::{Serialize, de::DeserializeOwned};

use crate::bare::{Browser, StopFn, register_service};

pub use minusmq::pub_sub::{Compression, Config, Keepalive};

/// Advertise a DNS-SD pub/sub service, sending a stream of T.
/// The service will be advertised until dropped.
pub struct PublisherService<T: Serialize> {
    stop: Option<StopFn>,
    publisher: minusmq::pub_sub::Publisher,
    send_buf: Vec<u8>,
    _msg_type: PhantomData<T>,
}

impl<T: Serialize> PublisherService<T> {
    /// Bind `port`, advertise it under `name`, and carry the stream `config`
    /// describes.
    pub fn new(name: &str, port: u16, config: Config) -> Result<Self> {
        let stop = register_service(name, port)?;
        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))?;
        let publisher = minusmq::pub_sub::Publisher::new(listener, config)?;
        Ok(Self {
            stop: Some(stop),
            publisher,
            send_buf: Vec::new(),
            _msg_type: PhantomData,
        })
    }

    /// Serialize a message and give it to every subscriber.
    ///
    /// The message is written into the buffer the one before it was written
    /// into, so a steady stream of messages of a settled size costs no
    /// allocation to send.
    pub fn send(&mut self, val: &T) -> Result<()> {
        self.send_buf.clear();
        postcard::to_io(val, &mut self.send_buf)?;
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

/// Browse for DNS-SD pub/sub services of one name, and connect subscribers to
/// them on request.
///
/// This is the half of the interface an application outside this workspace
/// reaches an advertised stream through.
pub struct SubscriberService<T: DeserializeOwned> {
    browser: Browser<SubConfig>,
    /// How this service's stream is carried, applied to every subscriber
    /// connected to it.
    config: Config,
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> SubscriberService<T> {
    /// Browse for publishers of the named service, whose stream `config`
    /// describes.
    /// Connect subscribers upon request.
    pub fn new(name: String, config: Config) -> Self {
        Self {
            browser: Browser::new(name, |service| {
                Ok(SubConfig {
                    hostname: service.hostname.clone(),
                    port: service.port,
                })
            }),
            config,
            _msg_type: PhantomData,
        }
    }

    /// List the services currently available.
    pub fn list(&self) -> Vec<String> {
        self.browser.list()
    }

    /// Connect a subscriber to the named service.
    pub fn subscribe(&self, name: &str) -> Result<Receiver<T>> {
        let config = self.config;
        self.browser
            .use_service(name, move |cfg| {
                // Resolve hostname to IP at subscribe time.
                let addr: SocketAddr = (&*cfg.hostname, cfg.port)
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Could not resolve {}:{}", cfg.hostname, cfg.port)
                    })?;
                Ok(Receiver::new(&addr.ip().to_string(), addr.port(), config))
            })
            .unwrap_or_else(|| bail!("no instance of service {} found", self.browser.name()))
    }
}

/// A strongly-typed TCP subscriber that expects messages to be encoded using
/// postcard.
pub struct Receiver<T: DeserializeOwned> {
    subscriber: minusmq::pub_sub::Subscriber,
    _msg_type: PhantomData<T>,
}

impl<T: DeserializeOwned> Receiver<T> {
    /// Create a new subscriber connected to the provided host:port, on the
    /// stream `config` describes.
    pub fn new(host: &str, port: u16, config: Config) -> Self {
        Self {
            subscriber: minusmq::pub_sub::Subscriber::new(host, port, config),
            _msg_type: PhantomData,
        }
    }

    /// Block until the next message arrives, and recover it. Yields nothing
    /// once the subscription has been stopped.
    ///
    /// The message is read out of the buffer it arrived in rather than out of
    /// a copy of it, so receiving costs the value and nothing else.
    pub fn receive_msg(&mut self) -> Result<Option<T>> {
        self.subscriber
            .recv()
            .map(|bytes| postcard::from_bytes(bytes).map_err(anyhow::Error::from))
            .transpose()
    }
}
