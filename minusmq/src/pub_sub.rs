//! TCP publish-subscribe.
//!
//! Publisher binds a port and accepts subscriber connections, and sends every
//! message to every subscriber connected at the time.
//!
//! Subscribers automatically reconnect on connection loss.

use anyhow::{Context, Result};
use log::{error, warn};
use socket2::{SockRef, TcpKeepalive};
use std::io::Write;
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::wire;

/// How long a connection goes without traffic before keepalive probing starts.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(5);

/// How long the keepalive probes wait between one another.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(2);

/// How many keepalive probes go unanswered before the connection is failed.
///
/// Together with [`KEEPALIVE_IDLE`] and [`KEEPALIVE_INTERVAL`] this puts a
/// silent peer's connection out of its misery about eleven seconds after the
/// traffic stops, against the operating system's default of a couple of hours.
const KEEPALIVE_RETRIES: u32 = 3;

/// Ask the operating system to fail a connection whose peer has stopped
/// answering, rather than hold it open indefinitely.
///
/// A peer that disappears without closing — an unplugged switch, a wireless
/// link that drops — leaves behind a socket that looks healthy and carries
/// nothing, and a thread reading from one waits on it forever. Keepalive
/// probes turn that silence into an error the connection can be remade from.
fn fail_a_silent_peer(socket: &TcpStream) -> std::io::Result<()> {
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL)
        .with_retries(KEEPALIVE_RETRIES);
    SockRef::from(socket).set_tcp_keepalive(&keepalive)
}

// --- Publisher ---

/// How long the accept loop waits for a subscriber before looking up.
///
/// A publisher on its way out releases the wait directly, so this is the
/// backstop for a release that never arrives — and the period at which
/// subscribers whose connections have failed are reaped.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a run of skipped messages goes unreported before its count is logged.
const SKIP_REPORT_PERIOD: Duration = Duration::from_secs(1);

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
    pending: Option<Arc<Vec<u8>>>,
    /// Messages replaced before the subscriber took them, since the last report.
    skipped: u64,
    /// When the last report of skipped messages was made, if any has been.
    reported_at: Option<Instant>,
    /// Whether the publisher has gone away. No further messages will arrive.
    closed: bool,
}

impl Mailbox {
    /// Create a mailbox, returning the handle that owns it and the shared
    /// reference the sender thread takes its work from.
    fn new() -> (MailboxHandle, Arc<Mailbox>) {
        let mailbox = Arc::new(Mailbox::default());
        (MailboxHandle(mailbox.clone()), mailbox)
    }

    /// Store `msg` as the message to send next, replacing any the subscriber
    /// has not taken, and report how many it has missed if a report is due.
    ///
    /// A subscriber that has stopped reading misses messages as fast as they
    /// are published, so they are counted and reported at most once per
    /// `SKIP_REPORT_PERIOD` rather than logged one apiece.
    fn post(&self, msg: Arc<Vec<u8>>, now: Instant) -> Option<u64> {
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
    fn take(&self) -> Option<Arc<Vec<u8>>> {
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
    /// thread finishes what it has and stops. Declaring it again changes
    /// nothing.
    ///
    /// A poisoned slot is closed as it stands rather than refused, so that
    /// closing cannot panic: it runs during teardown, where unwinding risks
    /// aborting the process.
    fn close(&self) {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed = true;
        self.posted.notify_all();
    }
}

/// Sole ownership of a mailbox, shared with the sender thread that empties it.
///
/// A mailbox posted to after its subscriber is gone is a message written
/// nowhere, so the right to post is held by exactly one place and cannot be
/// copied out of it.
struct MailboxHandle(Arc<Mailbox>);

impl std::ops::Deref for MailboxHandle {
    type Target = Mailbox;

    fn deref(&self) -> &Mailbox {
        &self.0
    }
}

/// A connected subscriber: the thread that writes to it, and the mailbox that
/// thread takes its work from.
///
/// A client owns its sender thread. Dropping one stops that thread and waits
/// for it, so a client that is gone has no thread left behind it.
struct Client {
    /// The address the subscriber connected from, to tell it apart in a log.
    peer: SocketAddr,
    mailbox: MailboxHandle,
    /// A second handle on the subscriber's socket. Shutting it down is what
    /// releases a sender thread parked in a write to a subscriber that has
    /// stopped reading.
    socket: TcpStream,
    /// The thread that writes to the subscriber. Empty once it has been taken
    /// out to be joined.
    sender: Option<JoinHandle<()>>,
}

impl Client {
    /// Whether the sender thread has stopped, which it does only when the
    /// connection has failed or the mailbox has been closed and emptied.
    fn is_finished(&self) -> bool {
        self.sender.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for Client {
    /// Stop the sender thread and wait for it.
    ///
    /// A sender thread parks in two places, and each needs its own release:
    /// closing the mailbox wakes one waiting for a message, and shutting the
    /// socket down fails the write of one blocked on a subscriber that has
    /// stopped reading. Both are done before the join, so the wait is only as
    /// long as it takes a released thread to return.
    fn drop(&mut self) {
        self.mailbox.close();
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(sender) = self.sender.take() {
            let _ = sender.join();
        }
    }
}

/// The publisher's subscribers, and the buffer their last message went out in.
#[derive(Default)]
struct Clients {
    /// Every subscriber currently connected.
    connected: Vec<Client>,
    /// The message published last, kept so that the next one can be written
    /// into it once nothing else holds it.
    spare: Option<Arc<Vec<u8>>>,
}

/// A message that could not be framed, and the spare buffer that framing it
/// did not use.
#[derive(Debug)]
struct Unframed {
    spare: Option<Arc<Vec<u8>>>,
    error: anyhow::Error,
}

/// Frame `data` into the buffer `spare` holds, or into a new one when a
/// subscriber is still holding that buffer.
///
/// Reusing a buffer is what keeps a steady stream of messages from allocating.
/// A buffer another thread still holds is a message a subscriber has not been
/// sent yet, so it is left alone and this message gets a buffer of its own.
///
/// A message that cannot be framed hands the buffer back rather than taking it
/// down with it, so that the message after it still has one to be written
/// into.
fn frame_into(spare: Option<Arc<Vec<u8>>>, data: &[u8]) -> Result<Arc<Vec<u8>>, Unframed> {
    let mut msg = spare.unwrap_or_default();
    if let Some(buf) = Arc::get_mut(&mut msg) {
        buf.clear();
        // Framing appends nothing when it fails, so what the buffer holds is
        // still nothing rather than half a message.
        return match wire::frame_msg_into(buf, data) {
            Ok(()) => Ok(msg),
            Err(error) => Err(Unframed {
                spare: Some(msg),
                error,
            }),
        };
    }
    match wire::frame_msg(data) {
        Ok(framed) => Ok(Arc::new(framed)),
        Err(error) => Err(Unframed {
            spare: Some(msg),
            error,
        }),
    }
}

/// A TCP-based publisher that pushes messages to connected subscribers.
///
/// Spawns a background accept thread. Subscribers connect and receive
/// length-prefixed messages, each on its own sender thread. A subscriber is
/// dropped when its connection fails, and never for being slow — a slow
/// subscriber misses messages instead.
pub struct Publisher {
    clients: Arc<Mutex<Clients>>,
    shutdown: Arc<AtomicBool>,
    /// The port the listener is bound to. Connecting to it is what releases
    /// the accept thread from its wait.
    port: u16,
    accept: Option<JoinHandle<()>>,
}

impl Publisher {
    /// Create a new publisher from an already-bound listener.
    /// Spawns a background thread to accept subscriber connections.
    pub fn new(listener: TcpListener) -> Result<Self> {
        let local_addr = listener.local_addr()?;
        // A dropped publisher releases the accept thread by connecting to this
        // listener itself. Bounding the wait is the backstop for a connection
        // that cannot be made: the thread then looks up on its own rather than
        // holding the drop open forever.
        if let Err(e) = SockRef::from(&listener).set_read_timeout(Some(ACCEPT_TIMEOUT)) {
            warn!("Failed to bound the wait for a subscriber: {e}");
        }
        log::debug!("pub_sub publisher listening on {local_addr}");

        let clients: Arc<Mutex<Clients>> = Arc::new(Mutex::new(Clients::default()));
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
            port: local_addr.port(),
            accept: Some(accept),
        })
    }

    /// Send data to every connected subscriber.
    ///
    /// Returns once every subscriber has been given the data; the writing
    /// happens on their own threads, so this never blocks on a socket. A
    /// subscriber that has not yet been sent the previous message misses it.
    pub fn send(&self, data: &[u8]) {
        // Reports are logged after the lock is released. The caller may be on
        // a deadline, and logging is not something to hold a lock across.
        let mut skip_reports = Vec::new();
        {
            let mut clients = self.clients.lock().unwrap();
            let Clients { connected, spare } = &mut *clients;
            // The message is framed once, however many subscribers share it,
            // and not at all if none do. Framing it here rather than on each
            // sender thread is what lets a thread write it in one call, so
            // that its length prefix shares a packet with its payload.
            if connected.is_empty() {
                return;
            }
            let msg = match frame_into(spare.take(), data) {
                Ok(msg) => msg,
                Err(Unframed {
                    spare: unused,
                    error,
                }) => {
                    error!("Dropping a message: {error:#}");
                    *spare = unused;
                    return;
                }
            };
            let now = Instant::now();
            for client in connected.iter() {
                if let Some(skipped) = client.mailbox.post(Arc::clone(&msg), now) {
                    skip_reports.push((client.peer, skipped));
                }
            }
            *spare = Some(msg);
        }
        for (peer, skipped) in skip_reports {
            warn!("Subscriber {peer} is behind: skipped {skipped} messages.");
        }
    }
}

impl Drop for Publisher {
    /// Stop and join every thread the publisher started.
    ///
    /// The accept thread goes first, so that no subscriber can be added after
    /// the ones present have been dealt with. Dropping the clients then stops
    /// every sender thread, including one parked in a write to a subscriber
    /// that is not reading.
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // A thread waiting for a subscriber is released by a subscriber, so
        // the publisher makes the last connection itself. The flag is set
        // first, so the thread sees it and drops that connection rather than
        // subscribing it.
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }

        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .connected
            .clear();
    }
}

fn accept_loop(listener: TcpListener, clients: Arc<Mutex<Clients>>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) => {
                // A publisher on its way out connects here to release the
                // wait. That connection carries no subscriber behind it.
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                match subscribe(stream, peer) {
                    Ok(client) => clients.lock().unwrap().connected.push(client),
                    Err(e) => warn!("Failed to subscribe a client: {e:#}"),
                }
            }
            Err(e) if is_timeout(&e) => (),
            Err(e) => {
                error!("pub_sub accept error: {e}");
                // An error that persists — a process out of file descriptors,
                // say — would otherwise spin this thread and its log as fast
                // as the two of them can go.
                thread::sleep(ACCEPT_TIMEOUT);
            }
        }
        // Reaping belongs on this thread rather than on the publisher's: it
        // happens between messages and nothing waits on it.
        reap_disconnected(&clients);
    }
}

/// Whether a failed accept is the wait running out rather than the listener
/// failing.
fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Start the thread that writes to a newly connected subscriber.
fn subscribe(stream: TcpStream, peer: SocketAddr) -> Result<Client> {
    if let Err(e) = stream.set_nodelay(true) {
        warn!("Failed to set TCP_NODELAY: {e}");
    }
    // A subscriber that vanishes without closing would otherwise keep its
    // sender thread and its place in the list for as long as the publisher
    // lives, taking a copy of every message published in the meantime.
    //
    // The probes only reach a connection that is idle, though, and a
    // connection carrying a stream of messages is not: with unacknowledged
    // data outstanding it is the retransmission timer that governs, and a
    // subscriber that vanishes mid-stream is held until that gives up.
    // `TCP_USER_TIMEOUT` would cover it. What it costs meanwhile is a client
    // that is not reaped and a run of skip warnings, which is the cheap half
    // of the problem: the half a show depends on is the subscriber's own end,
    // where a publisher that stops sending is idle by definition and the
    // probes do fire.
    if let Err(e) = fail_a_silent_peer(&stream) {
        warn!("Failed to set keepalive on subscriber {peer}: {e}");
    }
    // Deliberately no write timeout. A sender thread is free to block for as
    // long as its subscriber takes, and a timeout firing mid-message would
    // leave a partial message on the wire, desynchronizing the framing for
    // every message after it.

    let socket = stream
        .try_clone()
        .context("failed to duplicate the subscriber socket")?;
    let (mailbox, shared) = Mailbox::new();
    let sender = thread::Builder::new()
        .name(format!("pub_sub-send-{peer}"))
        .spawn(move || send_loop(stream, peer, &shared))
        .context("failed to spawn sender thread")?;

    log::debug!("Subscriber {peer} connected");
    Ok(Client {
        peer,
        mailbox,
        socket,
        sender: Some(sender),
    })
}

/// Write messages to one subscriber until its connection fails or the
/// publisher goes away.
///
/// Each message goes out in one write, so that its length prefix cannot reach
/// the subscriber as a packet of its own. `TCP_NODELAY` is what carries that
/// write promptly.
fn send_loop(mut stream: TcpStream, peer: SocketAddr, mailbox: &Mailbox) {
    while let Some(msg) = mailbox.take() {
        if let Err(e) = stream.write_all(&msg) {
            warn!("Dropping subscriber {peer}: {e}");
            return;
        }
    }
}

/// Forget the subscribers whose sender threads have stopped, which they do
/// only when the connection has failed.
fn reap_disconnected(clients: &Mutex<Clients>) {
    clients
        .lock()
        .unwrap()
        .connected
        .retain(|client| !client.is_finished());
}

// --- Subscriber ---

/// The buffer capacity a subscriber keeps between messages.
///
/// A message far larger than the usual one would otherwise pin a buffer its
/// size for the life of the connection. Capacity above this ceiling is
/// released once the message in it has been read, while a stream of ordinary
/// messages stays well beneath it and is received into the same allocation
/// every time.
const RETAINED_BUF_CAPACITY: usize = 64 * 1024;

/// A TCP-based subscriber that connects to a publisher and receives every
/// message it sends. Automatically reconnects on connection loss.
pub struct Subscriber {
    host: String,
    port: u16,
    /// The longest message this subscription carries. A length prefix
    /// claiming more than this fails the read, rather than reserving up to
    /// four gigabytes to match a publisher that is confused or hostile.
    max_msg_len: usize,
    stream: Option<TcpStream>,
    /// The message received last, refilled by the next one to arrive.
    buf: Vec<u8>,
}

impl Subscriber {
    /// Create a new subscriber that accepts messages of up to `max_msg_len`
    /// bytes. Does not connect immediately — connection happens lazily on the
    /// first `recv()` call.
    pub fn new(host: impl Into<String>, port: u16, max_msg_len: usize) -> Self {
        Subscriber {
            host: host.into(),
            port,
            max_msg_len,
            stream: None,
            buf: Vec::new(),
        }
    }

    /// Block until the next message arrives. Handles reconnection internally —
    /// if the connection drops, reconnects transparently.
    ///
    /// The message stands until the next one is received, and arrives in the
    /// buffer the one before it did, so a stream of messages of a settled size
    /// costs no allocation to receive.
    pub fn recv(&mut self) -> &[u8] {
        // Give back what an outsized message took, now that nothing holds it.
        if self.buf.capacity() > RETAINED_BUF_CAPACITY {
            self.buf.clear();
            self.buf.shrink_to(RETAINED_BUF_CAPACITY);
        }

        loop {
            // Ensure we have a connection.
            if self.stream.is_none() {
                self.connect();
            }

            // Try to read a message.
            match wire::read_msg_into(
                self.stream.as_mut().unwrap(),
                &mut self.buf,
                self.max_msg_len,
            ) {
                Ok(()) => break,
                Err(e) => {
                    // Connection lost — drop it and reconnect on next iteration.
                    warn!("Subscriber read error ({}:{}): {e}", self.host, self.port);
                    self.stream = None;
                }
            }
        }
        &self.buf
    }

    /// Connect to the publisher, retrying with backoff until successful.
    fn connect(&mut self) {
        let mut backoff = Duration::from_millis(100);
        let max_backoff = Duration::from_secs(5);

        loop {
            let addr = format!("{}:{}", self.host, self.port);
            match TcpStream::connect(&addr) {
                Ok(stream) => {
                    // Without this, a publisher that goes away without closing
                    // leaves this subscriber blocked in a read that no message
                    // and no error will ever end.
                    if let Err(e) = fail_a_silent_peer(&stream) {
                        warn!("Failed to set keepalive on the connection to {addr}: {e}");
                    }
                    log::debug!("Subscriber connected to {addr}");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    /// A limit no message in a test reaches, for the tests that are about
    /// something other than the limit.
    const TEST_MAX_MESSAGE_LEN: usize = 4 * 1024 * 1024;

    fn test_publisher() -> (Publisher, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let publisher = Publisher::new(listener).unwrap();
        // Give accept thread time to start.
        thread::sleep(Duration::from_millis(50));
        (publisher, port)
    }

    #[test]
    fn every_subscriber_receives_every_message() {
        let (publisher, port) = test_publisher();

        // Spawn two subscribers in threads since both need to recv().
        let handle1 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, TEST_MAX_MESSAGE_LEN);
            sub.recv().to_vec()
        });

        // Second subscriber in another thread.
        let port2 = port;
        let handle2 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port2, TEST_MAX_MESSAGE_LEN);
            sub.recv().to_vec()
        });

        // Give both subscribers time to connect.
        thread::sleep(Duration::from_millis(300));
        publisher.send(b"broadcast");

        let msg1 = handle1.join().unwrap();
        let msg2 = handle2.join().unwrap();
        assert_eq!(msg1, b"broadcast");
        assert_eq!(msg2, b"broadcast");
    }

    /// Publish `msg` until a subscriber reports receiving it.
    ///
    /// A subscription is taken up by the accept thread rather than by the
    /// connect that starts it, so the moment a message first has somewhere to
    /// go is not one a test can name. Republishing is free, and the messages
    /// this transport carries are idempotent.
    fn publish_until_received(publisher: &Publisher, msg: &[u8], receipts: &Receiver<Vec<u8>>) {
        /// How long a subscriber gets to receive a message published over and
        /// over, before it is taken not to be receiving at all.
        const LIMIT: Duration = Duration::from_secs(10);
        /// How long to wait on the receipts before publishing again.
        const REPUBLISH_AFTER: Duration = Duration::from_millis(50);

        let deadline = Instant::now() + LIMIT;
        while Instant::now() < deadline {
            publisher.send(msg);
            while let Ok(received) = receipts.recv_timeout(REPUBLISH_AFTER) {
                if received == msg {
                    return;
                }
            }
        }
        panic!(
            "no subscriber received {:?} in {LIMIT:?}",
            String::from_utf8_lossy(msg)
        );
    }

    /// A subscriber whose connection is closed under it reconnects and
    /// receives what is published next.
    ///
    /// A publisher that restarts, or a network that comes back, reaches a
    /// subscriber only through the reconnect inside `recv`. A subscriber that
    /// stays blocked instead is a client showing a frame from before the
    /// interruption until someone restarts the process.
    #[test]
    fn a_subscriber_reconnects_when_its_connection_is_closed() {
        const BEFORE: &[u8] = b"before";
        const AFTER: &[u8] = b"after";

        let (publisher, port) = test_publisher();
        let (received, receipts) = channel();
        let subscriber = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, TEST_MAX_MESSAGE_LEN);
            loop {
                let msg = sub.recv().to_vec();
                let is_last = msg == AFTER;
                if received.send(msg).is_err() || is_last {
                    return;
                }
            }
        });

        publish_until_received(&publisher, BEFORE, &receipts);

        // Closing the connection while the listener stays bound is what a
        // publisher losing a subscriber looks like from the subscriber's end,
        // whatever took the connection away.
        let client = publisher
            .clients
            .lock()
            .unwrap()
            .connected
            .pop()
            .expect("the subscriber never connected");
        drop(client);

        publish_until_received(&publisher, AFTER, &receipts);
        subscriber.join().unwrap();
    }

    /// A settled stream of messages is received into one allocation, and the
    /// capacity an outsized message takes is given back once it has been read.
    /// Every message arrives whole, however large it is and whatever buffer it
    /// lands in.
    ///
    /// Each message is published only after the one before it has been
    /// received, so a subscriber that is slow misses nothing here.
    #[test]
    fn subscriber_buffer_is_reused_and_released() {
        /// A message of the size the transport carries all day.
        const SETTLED_LEN: usize = 2_700;
        /// A message far larger than that.
        const OUTSIZED_LEN: usize = 1_000_000;
        /// The byte every message is made of, to recognize it by.
        const FILL: u8 = 0xAB;

        let (publisher, port) = test_publisher();
        let (requests, to_publish) = channel::<usize>();
        thread::spawn(move || {
            while let Ok(len) = to_publish.recv() {
                let msg = vec![FILL; len];
                publisher.send(&msg);
            }
        });

        let mut sub = Subscriber::new("127.0.0.1", port, TEST_MAX_MESSAGE_LEN);
        sub.connect();
        // Let the publisher's accept thread take up the subscription.
        thread::sleep(Duration::from_millis(200));

        let mut receive = |len: usize| -> usize {
            requests.send(len).unwrap();
            let received = sub.recv();
            assert_eq!(received.len(), len);
            assert!(
                received.iter().all(|&b| b == FILL),
                "a {len}-byte message arrived holding something other than what was published"
            );
            sub.buf.capacity()
        };

        let settled = receive(SETTLED_LEN);
        for _ in 0..5 {
            assert_eq!(
                receive(SETTLED_LEN),
                settled,
                "a settled stream of messages reallocated"
            );
        }

        assert!(
            receive(OUTSIZED_LEN) >= OUTSIZED_LEN,
            "an outsized message was received into a buffer too small to hold it"
        );
        let released = receive(SETTLED_LEN);
        assert!(
            released <= RETAINED_BUF_CAPACITY,
            "an outsized message left {released} bytes of capacity behind it"
        );
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
    fn stalled_subscriber(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
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
        let mut stalled = stalled_subscriber(port);

        let (received, receipts) = channel();
        thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, TEST_MAX_MESSAGE_LEN);
            loop {
                let msg = sub.recv().to_vec();
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
            publisher.send(&bulk);
            slowest_send = slowest_send.max(sent_at.elapsed());
        }
        publisher.send(LAST_MESSAGE);

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
            match wire::read_msg(&mut stalled, TEST_MAX_MESSAGE_LEN) {
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
        let _stalled = stalled_subscriber(port);
        thread::sleep(Duration::from_millis(300));

        let bulk = vec![0xABu8; LARGE_MESSAGE];
        for _ in 0..MESSAGES_TO_STALL {
            publisher.send(&bulk);
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
        let _stalled = stalled_subscriber(port);
        thread::sleep(Duration::from_millis(300));

        let bulk = vec![0xABu8; LARGE_MESSAGE];
        for _ in 0..MESSAGES_TO_STALL {
            publisher.send(&bulk);
        }

        // Take the client out of the list, as any unsubscribe path would.
        let client = publisher
            .clients
            .lock()
            .unwrap()
            .connected
            .pop()
            .expect("the subscriber never connected");
        assert!(
            !client.is_finished(),
            "the sender thread stopped on its own, before the client was dropped"
        );
        let mailbox = Arc::clone(&client.mailbox.0);

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

    /// A message framed into a buffer a subscriber still holds gets a buffer
    /// of its own, and neither message is disturbed.
    ///
    /// Reclaiming is an optimization and failing to reclaim is the ordinary
    /// case whenever a subscriber is mid-write, so the two paths must produce
    /// the same message.
    #[test]
    fn a_message_is_framed_the_same_way_whether_or_not_a_buffer_comes_back() {
        let held = frame_into(None, b"first").unwrap();
        let borrowed = Arc::clone(&held);
        let fresh = frame_into(Some(held), b"second").unwrap();
        assert_eq!(*borrowed, wire::frame_msg(b"first").unwrap());
        assert_eq!(*fresh, wire::frame_msg(b"second").unwrap());

        let reused = frame_into(Some(fresh), b"third").unwrap();
        assert_eq!(*reused, wire::frame_msg(b"third").unwrap());
    }
}
