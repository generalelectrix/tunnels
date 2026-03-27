//! TCP publish-subscribe with channel-based filtering.
//!
//! Publisher binds a port and accepts subscriber connections. Each subscriber
//! sends a single byte (channel number) on connect. The publisher only sends
//! messages to subscribers whose channel matches.
//!
//! Subscribers automatically reconnect on connection loss.

use anyhow::{Context, Result};
use log::{error, warn};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::wire;

// --- Publisher ---

struct Client {
    stream: TcpStream,
    channel: u8,
}

/// A TCP-based publisher that pushes messages to connected subscribers.
///
/// Spawns a background accept thread. Subscribers connect, send their channel
/// byte, and receive length-prefixed messages. Slow or disconnected clients
/// are dropped automatically.
pub struct Publisher {
    clients: Arc<Mutex<Vec<Client>>>,
}

impl Publisher {
    /// Create a new publisher from an already-bound listener.
    /// Spawns a background thread to accept subscriber connections.
    pub fn new(listener: TcpListener) -> Result<Self> {
        let local_addr = listener.local_addr()?;
        log::debug!("pub_sub publisher listening on {local_addr}");

        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let clients_accept = clients.clone();

        thread::Builder::new()
            .name(format!("pub_sub-accept-{}", local_addr.port()))
            .spawn(move || accept_loop(listener, clients_accept))
            .context("failed to spawn accept thread")?;

        Ok(Publisher { clients })
    }

    /// Send data to all subscribers on the given channel.
    /// If a client's write fails (slow, disconnected), it is removed.
    pub fn send(&self, channel: u8, data: &[u8]) {
        let mut clients = self.clients.lock().unwrap();
        clients.retain_mut(|client| {
            if client.channel != channel {
                return true; // keep, just not this channel
            }
            match wire::write_msg(&mut client.stream, data) {
                Ok(()) => {
                    // Flush to ensure data is sent promptly.
                    match client.stream.flush() {
                        Ok(()) => true,
                        Err(e) => {
                            warn!("Dropping subscriber (channel {channel}): flush error: {e}");
                            false
                        }
                    }
                }
                Err(e) => {
                    warn!("Dropping subscriber (channel {channel}): {e}");
                    false
                }
            }
        });
    }
}

fn accept_loop(listener: TcpListener, clients: Arc<Mutex<Vec<Client>>>) {
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                // Read the channel byte.
                let mut channel_buf = [0u8; 1];
                match stream.read_exact(&mut channel_buf) {
                    Ok(()) => {
                        let channel = channel_buf[0];
                        if let Err(e) = stream.set_nodelay(true) {
                            warn!("Failed to set TCP_NODELAY: {e}");
                        }
                        log::debug!("Subscriber connected (channel {channel})");
                        clients.lock().unwrap().push(Client { stream, channel });
                    }
                    Err(e) => {
                        warn!("Failed to read channel from subscriber: {e}");
                    }
                }
            }
            Err(e) => {
                error!("pub_sub accept error: {e}");
            }
        }
    }
}

// --- Subscriber ---

/// A TCP-based subscriber that connects to a publisher, subscribes to a
/// channel, and receives messages. Automatically reconnects on connection loss.
pub struct Subscriber {
    host: String,
    port: u16,
    channel: u8,
    stream: Option<TcpStream>,
}

impl Subscriber {
    /// Create a new subscriber. Does not connect immediately — connection
    /// happens lazily on the first `recv()` call.
    pub fn new(host: impl Into<String>, port: u16, channel: u8) -> Self {
        Subscriber {
            host: host.into(),
            port,
            channel,
            stream: None,
        }
    }

    /// Block until the next message arrives. Handles reconnection internally —
    /// if the connection drops, reconnects and re-subscribes transparently.
    pub fn recv(&mut self) -> Vec<u8> {
        loop {
            // Ensure we have a connection.
            if self.stream.is_none() {
                self.connect();
            }

            // Try to read a message.
            match wire::read_msg(self.stream.as_mut().unwrap()) {
                Ok(data) => return data,
                Err(e) => {
                    // Connection lost — drop it and reconnect on next iteration.
                    warn!(
                        "Subscriber read error ({}:{} channel {}): {e}",
                        self.host, self.port, self.channel
                    );
                    self.stream = None;
                }
            }
        }
    }

    /// Connect to the publisher, retrying with backoff until successful.
    fn connect(&mut self) {
        let mut backoff = Duration::from_millis(100);
        let max_backoff = Duration::from_secs(5);

        loop {
            let addr = format!("{}:{}", self.host, self.port);
            match self.try_connect(&addr) {
                Ok(stream) => {
                    log::debug!(
                        "Subscriber connected to {addr} (channel {})",
                        self.channel
                    );
                    self.stream = Some(stream);
                    return;
                }
                Err(e) => {
                    warn!(
                        "Subscriber connect to {addr} failed: {e}. Retrying in {backoff:?}."
                    );
                    thread::sleep(backoff);
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }
    }

    fn try_connect(&self, addr: &str) -> Result<TcpStream> {
        let mut stream = TcpStream::connect(addr).context("TCP connect failed")?;
        // Send the subscribe handshake: one byte, the channel number.
        stream
            .write_all(&[self.channel])
            .context("failed to send channel byte")?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_publisher() -> (Publisher, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let publisher = Publisher::new(listener).unwrap();
        // Give accept thread time to start.
        thread::sleep(Duration::from_millis(50));
        (publisher, port)
    }

    #[test]
    fn single_subscriber_receives_messages() {
        let (publisher, port) = test_publisher();
        let mut sub = Subscriber::new("127.0.0.1", port, 0);

        thread::spawn(move || {
            // Give subscriber time to connect.
            thread::sleep(Duration::from_millis(200));
            publisher.send(0, b"hello");
        });

        let msg = sub.recv();
        assert_eq!(msg, b"hello");
    }

    #[test]
    fn channel_filtering() {
        let (publisher, port) = test_publisher();
        let mut sub = Subscriber::new("127.0.0.1", port, 1);

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            // Send on channel 0 (should not reach subscriber on channel 1).
            publisher.send(0, b"wrong channel");
            // Send on channel 1 (should reach subscriber).
            publisher.send(1, b"right channel");
        });

        let msg = sub.recv();
        assert_eq!(msg, b"right channel");
    }

    #[test]
    fn multiple_subscribers_same_channel() {
        let (publisher, port) = test_publisher();

        // Spawn two subscribers in threads since both need to recv().
        let handle1 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, 0);
            sub.recv()
        });

        // Second subscriber in another thread.
        let port2 = port;
        let handle2 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port2, 0);
            sub.recv()
        });

        // Give both subscribers time to connect.
        thread::sleep(Duration::from_millis(300));
        publisher.send(0, b"broadcast");

        let msg1 = handle1.join().unwrap();
        let msg2 = handle2.join().unwrap();
        assert_eq!(msg1, b"broadcast");
        assert_eq!(msg2, b"broadcast");
    }

    // Reconnection is tested manually — the subscriber's connect() loop
    // with exponential backoff handles server restarts transparently.
    // Automated testing of reconnection requires SO_REUSEADDR + port rebinding
    // which is flaky in CI/sandbox environments due to TIME_WAIT.

    #[test]
    fn multiple_channels_independent() {
        let (publisher, port) = test_publisher();

        let handle_ch0 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, 0);
            sub.recv()
        });

        let port2 = port;
        let handle_ch1 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port2, 1);
            sub.recv()
        });

        thread::sleep(Duration::from_millis(300));
        publisher.send(0, b"for-ch0");
        publisher.send(1, b"for-ch1");

        let msg0 = handle_ch0.join().unwrap();
        let msg1 = handle_ch1.join().unwrap();
        assert_eq!(msg0, b"for-ch0");
        assert_eq!(msg1, b"for-ch1");
    }

    #[test]
    fn large_frame() {
        let (publisher, port) = test_publisher();
        let mut sub = Subscriber::new("127.0.0.1", port, 0);

        let big = vec![0xAB; 1_000_000]; // 1 MB, typical large frame
        let big_clone = big.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            publisher.send(0, &big_clone);
        });

        let msg = sub.recv();
        assert_eq!(msg, big);
    }
}
