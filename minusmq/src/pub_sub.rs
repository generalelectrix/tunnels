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
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::wire;

// --- Publisher ---

/// How often the accept loop looks up from the listener, to notice that the
/// publisher is gone and to reap subscribers whose connections have failed.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How long a connection has to name its channel before it is abandoned.
///
/// A subscriber sends its channel byte immediately after connecting, so this
/// only ever expires on a connection that is not one. Bounding it keeps a
/// connection that says nothing from wedging the accept loop, and with it the
/// publisher's shutdown.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a run of skipped messages goes unreported before its count is logged.
const SKIP_REPORT_PERIOD: Duration = Duration::from_secs(1);

/// How long a dropped publisher waits for its subscribers to be sent what they
/// have already been given.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(250);

/// The one message a subscriber has been given and not yet been sent.
///
/// The publisher stores a message and moves on; the subscriber's sender thread
/// takes it and writes it. A message the subscriber has not taken is replaced
/// rather than queued behind, so a subscriber that cannot keep up misses
/// messages instead of costing the publisher, and the subscribers waiting
/// behind it, the time it takes to catch up.
#[derive(Default)]
struct Mailbox {
    slot: Mutex<Slot>,
    posted: Condvar,
}

#[derive(Default)]
struct Slot {
    /// The message waiting to be sent, if there is one.
    pending: Option<Arc<[u8]>>,
    /// Messages replaced before the subscriber took them, since the last report.
    skipped: u64,
    /// When the last report of skipped messages was made, if any has been.
    reported_at: Option<Instant>,
    /// Whether the publisher has gone away. No further messages will arrive.
    closed: bool,
}

impl Mailbox {
    /// Store `msg` as the message to send next, replacing any the subscriber
    /// has not taken, and report how many it has missed if a report is due.
    ///
    /// A subscriber that has stopped reading misses messages as fast as they
    /// are published, so they are counted and reported at most once per
    /// `SKIP_REPORT_PERIOD` rather than logged one apiece.
    fn post(&self, msg: Arc<[u8]>, now: Instant) -> Option<u64> {
        let mut slot = self.slot.lock().unwrap();
        let replaced = slot.pending.replace(msg).is_some();
        self.posted.notify_one();
        if !replaced {
            return None;
        }
        slot.skipped += 1;
        match slot.reported_at {
            Some(reported_at) if now.duration_since(reported_at) < SKIP_REPORT_PERIOD => None,
            _ => {
                slot.reported_at = Some(now);
                Some(std::mem::take(&mut slot.skipped))
            }
        }
    }

    /// Block until there is a message to send, and take it.
    /// Yields `None` once the publisher is gone and the last message is taken.
    fn take(&self) -> Option<Arc<[u8]>> {
        let mut slot = self.slot.lock().unwrap();
        loop {
            if let Some(msg) = slot.pending.take() {
                return Some(msg);
            }
            if slot.closed {
                return None;
            }
            slot = self.posted.wait(slot).unwrap();
        }
    }

    /// Declare that no further messages will be posted, so that the sender
    /// thread finishes what it has and stops.
    fn close(&self) {
        self.slot.lock().unwrap().closed = true;
        self.posted.notify_all();
    }
}

/// A connected subscriber: the thread that writes to it, and the mailbox that
/// thread takes its work from.
struct Client {
    channel: u8,
    mailbox: Arc<Mailbox>,
    /// A second handle on the subscriber's socket. Shutting it down is what
    /// releases a sender thread parked in a write to a subscriber that has
    /// stopped reading.
    socket: TcpStream,
    sender: JoinHandle<()>,
}

/// A TCP-based publisher that pushes messages to connected subscribers.
///
/// Spawns a background accept thread. Subscribers connect, send their channel
/// byte, and receive length-prefixed messages, each on its own sender thread.
/// A subscriber is dropped when its connection fails, and never for being
/// slow — a slow subscriber misses messages instead.
pub struct Publisher {
    clients: Arc<Mutex<Vec<Client>>>,
    shutdown: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Publisher {
    /// Create a new publisher from an already-bound listener.
    /// Spawns a background thread to accept subscriber connections.
    pub fn new(listener: TcpListener) -> Result<Self> {
        let local_addr = listener.local_addr()?;
        // Accepting without blocking is what lets the accept thread notice
        // that the publisher has been dropped.
        listener
            .set_nonblocking(true)
            .context("failed to set the listener non-blocking")?;
        log::debug!("pub_sub publisher listening on {local_addr}");

        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let accept = thread::Builder::new()
            .name(format!("pub_sub-accept-{}", local_addr.port()))
            .spawn({
                let clients = clients.clone();
                let shutdown = shutdown.clone();
                move || accept_loop(listener, clients, shutdown)
            })
            .context("failed to spawn accept thread")?;

        Ok(Publisher {
            clients,
            shutdown,
            accept: Some(accept),
        })
    }

    /// Send data to all subscribers on the given channel.
    ///
    /// Returns once every subscriber on the channel has been given the data;
    /// the writing happens on their own threads, so this never blocks on a
    /// socket. A subscriber that has not yet been sent the previous message
    /// misses it.
    pub fn send(&self, channel: u8, data: &[u8]) {
        // Reports are logged after the lock is released. The caller may be on
        // a deadline, and logging is not something to hold a lock across.
        let mut skip_reports = Vec::new();
        {
            let clients = self.clients.lock().unwrap();
            let now = Instant::now();
            // The data is copied once, however many subscribers share it, and
            // not at all if none do.
            let mut msg: Option<Arc<[u8]>> = None;
            for client in clients.iter().filter(|c| c.channel == channel) {
                let msg = msg.get_or_insert_with(|| Arc::from(data));
                if let Some(skipped) = client.mailbox.post(Arc::clone(msg), now) {
                    skip_reports.push(skipped);
                }
            }
        }
        for skipped in skip_reports {
            warn!("Subscriber (channel {channel}) is behind: skipped {skipped} messages.");
        }
    }
}

impl Drop for Publisher {
    /// Stop and join every thread the publisher started.
    ///
    /// The accept thread goes first, so that no subscriber can be added after
    /// the ones present have been dealt with. Sender threads are then told
    /// that no more messages are coming and given a moment to write what they
    /// already have; whatever remains after that is parked in a write to a
    /// subscriber that is not reading, and only closing the socket will
    /// release it.
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }

        let mut clients = self.clients.lock().unwrap();
        for client in clients.iter() {
            client.mailbox.close();
        }
        let deadline = Instant::now() + FLUSH_TIMEOUT;
        while Instant::now() < deadline && clients.iter().any(|c| !c.sender.is_finished()) {
            thread::sleep(Duration::from_millis(1));
        }
        for client in clients.drain(..) {
            let _ = client.socket.shutdown(Shutdown::Both);
            let _ = client.sender.join();
        }
    }
}

fn accept_loop(listener: TcpListener, clients: Arc<Mutex<Vec<Client>>>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => match subscribe(stream) {
                Ok(client) => clients.lock().unwrap().push(client),
                Err(e) => warn!("Failed to subscribe a client: {e:#}"),
            },
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    error!("pub_sub accept error: {e}");
                }
                // Reaping belongs on this thread rather than on the publisher's:
                // it happens between messages and nothing waits on it.
                reap_disconnected(&clients);
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
        }
    }
}

/// Complete a subscriber's handshake and start the thread that writes to it.
fn subscribe(mut stream: TcpStream) -> Result<Client> {
    // The listener is non-blocking so that it can be shut down; the
    // connections it yields are used blocking, on their own threads.
    stream
        .set_nonblocking(false)
        .context("failed to set the connection blocking")?;
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .context("failed to set the handshake read timeout")?;

    let mut channel_buf = [0u8; 1];
    stream
        .read_exact(&mut channel_buf)
        .context("failed to read channel from subscriber")?;
    let channel = channel_buf[0];

    if let Err(e) = stream.set_nodelay(true) {
        warn!("Failed to set TCP_NODELAY: {e}");
    }
    // Deliberately no write timeout. A sender thread is free to block for as
    // long as its subscriber takes, and a timeout firing mid-message would
    // leave a partial message on the wire, desynchronizing the framing for
    // every message after it.

    let socket = stream
        .try_clone()
        .context("failed to duplicate the subscriber socket")?;
    let mailbox = Arc::new(Mailbox::default());
    let sender = thread::Builder::new()
        .name(format!("pub_sub-send-ch{channel}"))
        .spawn({
            let mailbox = mailbox.clone();
            move || send_loop(stream, channel, &mailbox)
        })
        .context("failed to spawn sender thread")?;

    log::debug!("Subscriber connected (channel {channel})");
    Ok(Client {
        channel,
        mailbox,
        socket,
        sender,
    })
}

/// Write messages to one subscriber until its connection fails or the
/// publisher goes away.
fn send_loop(mut stream: TcpStream, channel: u8, mailbox: &Mailbox) {
    while let Some(msg) = mailbox.take() {
        if let Err(e) = write_msg(&mut stream, &msg) {
            warn!("Dropping subscriber (channel {channel}): {e:#}");
            return;
        }
    }
}

fn write_msg(stream: &mut TcpStream, msg: &[u8]) -> Result<()> {
    wire::write_msg(stream, msg)?;
    // Flush to ensure data is sent promptly.
    stream.flush().context("flush error")?;
    Ok(())
}

/// Forget the subscribers whose sender threads have stopped, which they do
/// only when the connection has failed.
fn reap_disconnected(clients: &Mutex<Vec<Client>>) {
    let mut clients = clients.lock().unwrap();
    let mut i = 0;
    while i < clients.len() {
        if clients[i].sender.is_finished() {
            let _ = clients.swap_remove(i).sender.join();
        } else {
            i += 1;
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
                    log::debug!("Subscriber connected to {addr} (channel {})", self.channel);
                    self.stream = Some(stream);
                    return;
                }
                Err(e) => {
                    warn!("Subscriber connect to {addr} failed: {e}. Retrying in {backoff:?}.");
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
    use std::sync::mpsc::channel;
    use std::time::Instant;

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

    /// A message large enough that a handful of them fill a socket's buffers.
    const LARGE_MESSAGE: usize = 1 << 20;

    /// Enough large messages to overrun the buffers of a subscriber that is
    /// not reading, in both directions.
    const MESSAGES_TO_STALL: usize = 32;

    /// The last message of the stall test, told apart from the bulk before it.
    const LAST_MESSAGE: &[u8] = b"still-subscribed";

    /// Publishing costs a copy and a wakeup, and nothing like this long.
    const SEND_LIMIT: Duration = Duration::from_millis(50);

    /// Connect a subscriber that never reads a byte of what it is sent.
    fn stalled_subscriber(port: u16, channel: u8) -> TcpStream {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(&[channel]).unwrap();
        stream.flush().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    /// A subscriber that stops reading misses messages and stays subscribed.
    ///
    /// Blocking the publisher on one subscriber's socket stalls every other
    /// subscriber with it, and dropping that subscriber for being slow costs
    /// it everything until it reconnects. Both cost more than the messages it
    /// misses, which the messages that follow supersede.
    #[test]
    fn a_subscriber_that_stops_reading_costs_only_itself() {
        let (publisher, port) = test_publisher();

        // Connect the subscriber that stalls first, so that a publisher
        // writing to its subscribers in turn reaches it before the other.
        let mut stalled = stalled_subscriber(port, 0);

        let (received, receipts) = channel();
        thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, 0);
            loop {
                let msg = sub.recv();
                let is_last = msg == LAST_MESSAGE;
                if received.send(msg).is_err() || is_last {
                    return;
                }
            }
        });
        // Give both subscribers time to connect.
        thread::sleep(Duration::from_millis(300));

        let bulk = vec![0xABu8; LARGE_MESSAGE];
        let mut slowest_send = Duration::ZERO;
        for _ in 0..MESSAGES_TO_STALL {
            let sent_at = Instant::now();
            publisher.send(0, &bulk);
            slowest_send = slowest_send.max(sent_at.elapsed());
        }
        publisher.send(0, LAST_MESSAGE);

        // The subscriber that kept reading was not held up by the one that
        // stopped: it still gets everything sent after the stall.
        let mut reader_reached_last = false;
        while let Ok(msg) = receipts.recv_timeout(Duration::from_secs(10)) {
            if msg == LAST_MESSAGE {
                reader_reached_last = true;
                break;
            }
        }
        assert!(
            reader_reached_last,
            "a subscriber that kept reading never received the last message"
        );

        // The stalled subscriber missed messages while it was not reading, but
        // it is still subscribed, so reading again reaches the last one sent.
        let mut stalled_reached_last = false;
        for _ in 0..=MESSAGES_TO_STALL {
            match wire::read_msg(&mut stalled) {
                Ok(msg) if msg == LAST_MESSAGE => {
                    stalled_reached_last = true;
                    break;
                }
                Ok(_) => (),
                Err(e) => panic!("the stalled subscriber was dropped: {e}"),
            }
        }
        assert!(
            stalled_reached_last,
            "the stalled subscriber never caught up to the last message"
        );

        assert!(
            slowest_send < SEND_LIMIT,
            "publishing blocked for {slowest_send:?} on a subscriber that stopped reading"
        );
    }

    /// A dropped publisher takes its threads with it.
    ///
    /// Dropping joins every thread the publisher started, so a drop that
    /// returns is proof that none of them are left running — including the
    /// sender thread of a subscriber that stopped reading, which is parked in
    /// a write that only closing its socket releases.
    #[test]
    fn dropping_the_publisher_stops_its_threads() {
        let (publisher, port) = test_publisher();
        // Held open for the duration, so that the sender thread writing to it
        // is blocked rather than merely finished.
        let _stalled = stalled_subscriber(port, 0);
        thread::sleep(Duration::from_millis(300));

        let bulk = vec![0xABu8; LARGE_MESSAGE];
        for _ in 0..MESSAGES_TO_STALL {
            publisher.send(0, &bulk);
        }

        let (dropped, completion) = channel();
        thread::spawn(move || {
            drop(publisher);
            let _ = dropped.send(());
        });
        assert!(
            completion.recv_timeout(Duration::from_secs(10)).is_ok(),
            "dropping the publisher never finished: a thread it started is still running"
        );
    }

    /// A client removed from the publisher's list takes its sender thread
    /// with it.
    ///
    /// A client owns the thread that writes to its subscriber, so dropping one
    /// must stop that thread — including a thread parked in a write to a
    /// subscriber that has stopped reading, which only shutting the socket
    /// down releases. The thread holds the client's mailbox for as long as it
    /// runs, so the mailbox outliving the drop is the thread outliving it.
    #[test]
    fn dropping_a_client_stops_its_sender_thread() {
        let (publisher, port) = test_publisher();
        // Held open for the duration, so that the sender thread writing to it
        // is blocked rather than merely finished.
        let _stalled = stalled_subscriber(port, 0);
        thread::sleep(Duration::from_millis(300));

        let bulk = vec![0xABu8; LARGE_MESSAGE];
        for _ in 0..MESSAGES_TO_STALL {
            publisher.send(0, &bulk);
        }

        // Take the client out of the list, as any unsubscribe path would.
        let client = publisher
            .clients
            .lock()
            .unwrap()
            .pop()
            .expect("the subscriber never connected");
        assert!(
            !client.sender.is_finished(),
            "the sender thread stopped on its own, before the client was dropped"
        );
        let mailbox = Arc::clone(&client.mailbox);

        let (dropped, completion) = channel();
        thread::spawn(move || {
            drop(client);
            let _ = dropped.send(());
        });
        assert!(
            completion.recv_timeout(Duration::from_secs(10)).is_ok(),
            "dropping the client never finished"
        );
        assert_eq!(
            Arc::strong_count(&mailbox),
            1,
            "the sender thread outlived the client that owned it"
        );
    }
}
