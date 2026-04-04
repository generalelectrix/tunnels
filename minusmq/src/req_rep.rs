//! One-shot TCP request-response.
//!
//! Each request opens a fresh TCP connection, sends a length-prefixed message,
//! reads a length-prefixed response, and closes. No persistent connections,
//! no cross-thread socket issues.

use anyhow::{Context, Result};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::wire;

/// Run a request-response server on an already-bound listener.
/// Reads one request per connection, calls `handler`, sends the response,
/// and closes the connection.
///
/// Runs forever (until the process exits or an unrecoverable error occurs).
pub fn serve<F>(listener: TcpListener, mut handler: F) -> Result<()>
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
                if let Err(e) = handle_one(&mut stream, &mut handler) {
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

fn handle_one<F>(stream: &mut TcpStream, handler: &mut F) -> Result<()>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    let request = wire::read_msg(stream).context("reading request")?;
    let response = handler(&request);
    wire::write_msg(stream, &response).context("writing response")?;
    Ok(())
}

/// Send a request and receive a response. Opens a fresh TCP connection,
/// sends the message, reads the response, and closes.
pub fn send(addr: impl ToSocketAddrs, msg: &[u8]) -> Result<Vec<u8>> {
    let mut stream = TcpStream::connect(addr).context("failed to connect")?;
    wire::write_msg(&mut stream, msg).context("writing request")?;
    wire::read_msg(&mut stream).context("reading response")
}

/// Like `send`, but with a timeout on the connection and the read/write.
/// The connect phase uses a shorter timeout (capped at 3s) since it should
/// complete almost instantly on a LAN.
pub fn send_with_timeout(
    addr: impl ToSocketAddrs,
    msg: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>> {
    const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
    let connect_timeout = timeout.min(MAX_CONNECT_TIMEOUT);

    // Resolve to a concrete SocketAddr so we can use connect_timeout.
    let socket_addr = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("address resolved to nothing"))?;
    let mut stream =
        TcpStream::connect_timeout(&socket_addr, connect_timeout).context("failed to connect")?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    wire::write_msg(&mut stream, msg).context("writing request")?;
    wire::read_msg(&mut stream).context("reading response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn serve_echo() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            serve(listener, |req| req.to_vec()).unwrap();
        });
        addr
    }

    #[test]
    fn basic_echo() {
        let addr = serve_echo();
        let response = send(addr, b"hello").unwrap();
        assert_eq!(response, b"hello");
    }

    #[test]
    fn handler_transforms() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            serve(listener, |req| {
                let mut r = req.to_vec();
                r.reverse();
                r
            })
            .unwrap();
        });

        thread::sleep(Duration::from_millis(50));
        let response = send(addr, b"abcd").unwrap();
        assert_eq!(response, b"dcba");
    }

    #[test]
    fn multiple_sequential_requests() {
        let addr = serve_echo();
        thread::sleep(Duration::from_millis(50));

        for i in 0..5 {
            let msg = format!("msg-{i}");
            let response = send(addr, msg.as_bytes()).unwrap();
            assert_eq!(response, msg.as_bytes());
        }
    }

    #[test]
    fn large_payload() {
        let addr = serve_echo();
        thread::sleep(Duration::from_millis(50));

        let big = vec![0xAB; 2_000_000]; // 2 MB
        let response = send(addr, &big).unwrap();
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
        let result = send_with_timeout(addr, b"hello", Duration::from_millis(100));
        assert!(result.is_err());
    }
}
