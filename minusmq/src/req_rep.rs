//! One-shot TCP request-response.
//!
//! Each request opens a fresh TCP connection, sends a length-prefixed message,
//! reads a length-prefixed response, and closes. No persistent connections,
//! no cross-thread socket issues.

use anyhow::{Context, Result};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::{DEFAULT_MAX_MESSAGE_LEN, wire};

/// The longest a request may wait to connect, however long it is willing to
/// wait for the response.
///
/// A connection is made or refused in a millisecond on a working LAN, so a
/// wait beyond this is a peer that is not there. Failing that fast leaves the
/// rest of the wait for the exchange itself, rather than spending it on a
/// connection an operating system would go on attempting for a minute or
/// more, by a margin that differs between them.
const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// How a request-response exchange is carried.
///
/// Each end is configured on its own: a server states what it will accept and
/// how long it will hold a connection open, and a client states what it will
/// accept and how long it will wait for it.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// The longest message this end accepts. A length prefix claiming more
    /// than this fails the exchange, rather than reserving memory to match a
    /// peer that is confused or hostile.
    pub max_message_len: usize,
    /// How long an accepted connection may go without moving a byte, in
    /// either direction, before it is abandoned.
    ///
    /// The timeout bounds each read and each write rather than the whole
    /// exchange, so it covers the longest gap between packets rather than the
    /// time a transfer takes: a request of hundreds of megabytes crosses a LAN
    /// well within it, while a connection that carries nothing at all costs
    /// this long and no more.
    pub connection_timeout: Duration,
    /// How long a request waits on its response, or unbounded when empty.
    pub request_timeout: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_message_len: DEFAULT_MAX_MESSAGE_LEN,
            connection_timeout: Duration::from_secs(30),
            request_timeout: None,
        }
    }
}

/// Run a request-response server on an already-bound listener.
/// Reads one request per connection, calls `handler`, sends the response,
/// and closes the connection.
///
/// A request declaring more than the configured message length is refused
/// before anything is sized to it, so a length prefix from a client that is
/// confused or hostile costs one connection rather than the memory it asked
/// for.
///
/// Requests are served one at a time, on the calling thread, so a client that
/// goes quiet holds up every request behind it. The configured connection
/// timeout bounds each read and each write rather than the whole exchange, so
/// a connection that carries nothing at all costs that long and no more, while
/// a peer that moves a byte just inside it holds the loop for as long as it
/// cares to.
///
/// Runs forever (until the process exits or an unrecoverable error occurs).
pub fn serve<F>(listener: TcpListener, config: Config, mut handler: F) -> Result<()>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    match listener.local_addr() {
        Ok(addr) => log::debug!("req_rep server listening on {addr}"),
        Err(e) => log::warn!("req_rep server started but could not determine local address: {e}"),
    }

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let bounded = stream
                    .set_read_timeout(Some(config.connection_timeout))
                    .and_then(|()| stream.set_write_timeout(Some(config.connection_timeout)));
                if let Err(e) = bounded {
                    log::warn!("req_rep could not bound a connection: {e}");
                }
                if let Err(e) = handle_one(&mut stream, config.max_message_len, &mut handler) {
                    log::error!("req_rep handler error: {e:#}");
                }
            }
            Err(e) => {
                log::error!("req_rep accept error: {e}");
            }
        }
    }

    Ok(())
}

fn handle_one<F>(stream: &mut TcpStream, max_request_len: usize, handler: &mut F) -> Result<()>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    let request = wire::read_msg(stream, max_request_len).context("reading request")?;
    let response = handler(&request);
    wire::write_msg(stream, &response).context("writing response")?;
    Ok(())
}

/// Send a request and receive a response. Opens a fresh TCP connection,
/// sends the message, reads the response, and closes.
///
/// A response declaring more than the configured message length is refused
/// before anything is sized to it. A request that is given a timeout waits no
/// longer than that for each step of the exchange; one that is not waits as
/// long as the operating system does.
pub fn send(addr: impl ToSocketAddrs, msg: &[u8], config: Config) -> Result<Vec<u8>> {
    let mut stream = match config.request_timeout {
        None => TcpStream::connect(addr).context("failed to connect")?,
        Some(timeout) => {
            // Resolve to a concrete SocketAddr so we can use connect_timeout.
            let socket_addr = addr
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| anyhow::anyhow!("address resolved to nothing"))?;
            let stream = TcpStream::connect_timeout(&socket_addr, timeout.min(MAX_CONNECT_TIMEOUT))
                .context("failed to connect")?;
            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))?;
            stream
        }
    };
    wire::write_msg(&mut stream, msg).context("writing request")?;
    wire::read_msg(&mut stream, config.max_message_len).context("reading response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// A limit no message in a test reaches, for the tests that are about
    /// something other than the limit.
    const TEST_MAX_MESSAGE_LEN: usize = 64 * 1024 * 1024;

    /// An exchange of the size a test sends, waiting as long as it takes.
    fn test_config() -> Config {
        Config {
            max_message_len: TEST_MAX_MESSAGE_LEN,
            ..Default::default()
        }
    }

    fn serve_echo() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            serve(listener, test_config(), |req| req.to_vec()).unwrap();
        });
        addr
    }

    #[test]
    fn handler_transforms() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            serve(listener, test_config(), |req| {
                let mut r = req.to_vec();
                r.reverse();
                r
            })
            .unwrap();
        });

        thread::sleep(Duration::from_millis(50));
        let response = send(addr, b"abcd", test_config()).unwrap();
        assert_eq!(response, b"dcba");
    }

    #[test]
    fn multiple_sequential_requests() {
        let addr = serve_echo();
        thread::sleep(Duration::from_millis(50));

        for i in 0..5 {
            let msg = format!("msg-{i}");
            let response = send(addr, msg.as_bytes(), test_config()).unwrap();
            assert_eq!(response, msg.as_bytes());
        }
    }

    #[test]
    fn large_payload() {
        let addr = serve_echo();
        thread::sleep(Duration::from_millis(50));

        let big = vec![0xAB; 2_000_000]; // 2 MB
        let response = send(addr, &big, test_config()).unwrap();
        assert_eq!(response, big);
    }

    #[test]
    fn timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // Server that accepts but never responds.
        thread::spawn(move || {
            for stream in listener.incoming() {
                let _stream = stream.unwrap();
                thread::sleep(Duration::from_secs(60));
            }
        });

        thread::sleep(Duration::from_millis(50));
        let result = send(
            addr,
            b"hello",
            Config {
                request_timeout: Some(Duration::from_millis(100)),
                ..test_config()
            },
        );
        assert!(result.is_err());
    }
}
