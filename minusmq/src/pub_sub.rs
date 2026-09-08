//! TCP publish-subscribe.
//!
//! A publisher is handed a bound listener, accepts subscriber connections on a
//! thread of its own, and gives every message to every subscriber connected at
//! the time.
//!
//! Subscribers automatically reconnect on connection loss.

use anyhow::{Context, Result};
use log::{error, info, warn};
use socket2::{SockRef, TcpKeepalive};
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{DEFAULT_MAX_MESSAGE_LEN, compress, wire};

/// When the operating system gives up on a peer that has stopped answering.
///
/// A peer that disappears without closing — an unplugged switch, a wireless
/// link that drops — leaves behind a socket that looks healthy and carries
/// nothing, and a thread reading from one waits on it forever. Probing turns
/// that silence into an error the connection can be remade from.
#[derive(Debug, Clone, Copy)]
pub struct Keepalive {
    /// How long a connection goes without traffic before probing starts.
    pub idle: Duration,
    /// How long the probes wait between one another.
    pub interval: Duration,
    /// How many probes go unanswered before the connection is failed.
    pub retries: u32,
}

impl Default for Keepalive {
    /// Put a silent peer's connection out of its misery about eleven seconds
    /// after the traffic stops, against the operating system's default of a
    /// couple of hours.
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(5),
            interval: Duration::from_secs(2),
            retries: 3,
        }
    }
}

/// How a stream's payloads are carried.
///
/// Compression is a property of a stream rather than of a message: nothing on
/// the wire says a payload is compressed, so a publisher writes what this says
/// and the subscribers of that stream are configured to read the same thing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Compression {
    /// Payloads are carried exactly as they are given.
    #[default]
    Plain,
    /// Payloads are compressed, and no message expands into more than
    /// `max_decompressed_len` bytes, which a subscriber enforces as it expands
    /// each message. A publisher compresses without consulting it.
    ///
    /// That bound is a second one, beyond the length a message may arrive at:
    /// a small compressed payload can declare an enormous expansion, and a
    /// declared length is not a reason to ask the allocator for one.
    Lz4 { max_decompressed_len: usize },
}

/// How a publish-subscribe stream is carried.
///
/// The two ends of a stream are configured apart from one another and have to
/// be told the same thing about what it carries. Everything else here is one
/// end's own business, so each field states where it takes effect: a value set
/// on the end it does not concern is ignored rather than obeyed.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// The longest message this stream carries, enforced on receive. A length
    /// prefix claiming more than this fails the read, rather than reserving
    /// memory to match a publisher that is confused or hostile. A publisher
    /// does not measure what it sends against it.
    pub max_message_len: usize,
    /// Whether the payloads on this stream are compressed. The one thing both
    /// ends have to be told the same: a publisher writes what this says and a
    /// subscriber reads what it says.
    pub compression: Compression,
    /// When a connection whose peer has gone silent is failed. Applied
    /// wherever a socket is opened, so it takes effect at both ends.
    pub keepalive: Keepalive,
    /// How long a publisher's accept loop sleeps between passes. A subscriber
    /// has no listener and ignores it.
    ///
    /// The loop asks whether anyone has connected rather than waiting for
    /// someone to, so one interval bounds three things: how long a connecting
    /// subscriber waits to be accepted, how long a departed one stays on the
    /// list after its sender thread has failed, and how long a publisher being
    /// dropped waits for its accept thread to notice. A subscriber connects
    /// once and stays for the length of a show, so the first of those is the
    /// cheapest of the three to pay for the other two.
    ///
    /// It asks rather than waits because a receive timeout bounds a receive,
    /// and whether an `accept` counts as one is the platform's business. On
    /// macOS it does not, and a loop waiting on the listener comes round only
    /// when somebody connects — so nothing is reaped between connections, on
    /// the platform a console publishes from.
    pub poll_interval: Duration,
    /// How long a write to a subscriber may make no progress before that
    /// subscriber is dropped. A subscriber writes to nobody and ignores it.
    ///
    /// A subscriber that goes away without closing leaves a connection that
    /// does not fail: its socket accepts writes until the buffers behind it
    /// fill, and the write after that neither completes nor fails for as long
    /// as the operating system goes on retransmitting. A deadline the
    /// publisher sets is what ends that wait, and ends it the same way on
    /// every platform.
    ///
    /// It bounds one write to the socket rather than one message, and
    /// `write_all` retries a short write, so failing a message takes as many
    /// deadlines as it takes writes: two on macOS and three on Linux, against
    /// a peer that has stopped reading.
    ///
    /// It is also the smaller half of what a departed subscriber costs, since
    /// the write it fails is the one that finds the buffers already full.
    /// Those are autotuned and no setting here shortens them: about 2.6 MB on
    /// Linux, which a stream of 4 kB messages at 240 Hz fills in 3.15s, for a
    /// subscriber dropped 5.84s after it departed against the 2s set here; and
    /// about 786 kB on macOS, a third of that, so the filling term there is
    /// proportionally smaller and detection correspondingly quicker.
    ///
    /// Dropping a subscriber early is the cheaper mistake: one that is still
    /// there reconnects on its own, while one that is not costs a thread, a
    /// copy of every message published, and a report of what it missed every
    /// [`SKIP_REPORT_PERIOD`] for as long as the publisher lives.
    pub write_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_message_len: DEFAULT_MAX_MESSAGE_LEN,
            compression: Compression::default(),
            keepalive: Keepalive::default(),
            poll_interval: Duration::from_millis(100),
            write_timeout: Duration::from_secs(2),
        }
    }
}

/// Ask the operating system to fail a connection whose peer has stopped
/// answering, rather than hold it open indefinitely.
fn fail_a_silent_peer(socket: &TcpStream, keepalive: Keepalive) -> std::io::Result<()> {
    let probes = TcpKeepalive::new()
        .with_time(keepalive.idle)
        .with_interval(keepalive.interval)
        .with_retries(keepalive.retries);
    SockRef::from(socket).set_tcp_keepalive(&probes)
}

// --- Publisher ---

/// How long a run of skipped messages goes unreported before its count is logged.
const SKIP_REPORT_PERIOD: Duration = Duration::from_secs(1);

/// The rate of missed messages, in messages per second, that separates a
/// subscriber dropping the occasional one from a subscriber that has stopped
/// keeping up.
///
/// Below it a subscriber is still showing the stream and losing the odd
/// message out of it, which is what this transport is for and not news. Above
/// it, what it shows is a fraction of what was published, which is a fault
/// worth putting in front of whoever is running the show.
const CONCERNING_SKIP_RATE: f64 = 5.0;

/// A run of messages a subscriber missed, and the stretch of time it missed
/// them over.
#[derive(Debug, Clone, Copy)]
struct Skips {
    /// How many messages were replaced before the subscriber took them.
    count: u64,
    /// How long the run they were counted in ran for, measured from the
    /// message that opened it. At least [`SKIP_REPORT_PERIOD`], and longer
    /// when the messages that would have closed it sooner were not published.
    over: Duration,
}

impl Skips {
    /// Whether these were missed faster than [`CONCERNING_SKIP_RATE`].
    ///
    /// The comparison is scaled rather than divided so that a run measured
    /// over no time at all is concerning, rather than an infinity.
    fn is_concerning(&self) -> bool {
        self.count as f64 > CONCERNING_SKIP_RATE * self.over.as_secs_f64()
    }
}

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
    /// Messages replaced before the subscriber took them, in the run now
    /// being counted.
    skipped: u64,
    /// When the message that opened the run now being counted was posted, if
    /// a run is open. A run opens on the first message the subscriber misses
    /// and closes when it is reported, so this anchors the run itself rather
    /// than the gap since whatever was reported before it.
    skipping_since: Option<Instant>,
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
    /// has not taken, and report what it has missed if a run of missed
    /// messages has been open for [`SKIP_REPORT_PERIOD`].
    ///
    /// A subscriber that has stopped reading misses messages as fast as they
    /// are published, so they are counted and reported once a period rather
    /// than logged one apiece. What closes a run is the period elapsing, not
    /// the next message missed: a burst that ends inside a period would
    /// otherwise wait for a miss that may be hours away, and be reported
    /// against a stretch of time it did not occupy.
    fn post(&self, msg: Arc<Vec<u8>>, now: Instant) -> Option<Skips> {
        let mut slot = self.slot.lock().unwrap();
        let replaced = slot.pending.replace(msg).is_some();
        self.posted.notify_one();
        if replaced {
            slot.skipped += 1;
            slot.skipping_since.get_or_insert(now);
        }
        let over = now.duration_since(slot.skipping_since?);
        if over < SKIP_REPORT_PERIOD {
            return None;
        }
        slot.skipping_since = None;
        Some(Skips {
            count: std::mem::take(&mut slot.skipped),
            over,
        })
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

/// The publisher's subscribers, and the buffers their last message went out
/// in.
#[derive(Default)]
struct Clients {
    /// Every subscriber currently connected.
    connected: Vec<Client>,
    /// The message published last, kept so that the next one can be written
    /// into it once nothing else holds it.
    spare: Option<Arc<Vec<u8>>>,
    /// The scratch a message is compressed in, ahead of framing. Untouched on
    /// a publisher whose stream carries its payloads as they are.
    compressed: Vec<u8>,
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
/// dropped when a write to it fails, which a write making no progress within
/// [`Config::write_timeout`] is one way to do: a peer that has departed
/// without closing and a peer too slow to accept a message are both dropped
/// that way, and are not told apart.
///
/// What a subscriber is never dropped for is missing messages. It misses them
/// while a write to it is outstanding, and a write that completes leaves it
/// subscribed however many it missed in the meantime.
pub struct Publisher {
    clients: Arc<Mutex<Clients>>,
    shutdown: Arc<AtomicBool>,
    /// How this stream carries its payloads. A message is compressed, if it is
    /// compressed at all, before it is framed.
    compression: Compression,
    accept: Option<JoinHandle<()>>,
}

impl Publisher {
    /// Create a new publisher from an already-bound listener.
    /// Spawns a background thread to accept subscriber connections.
    pub fn new(listener: TcpListener, config: Config) -> Result<Self> {
        let local_addr = listener.local_addr()?;
        log::debug!("pub_sub publisher listening on {local_addr}");

        let clients: Arc<Mutex<Clients>> = Arc::new(Mutex::new(Clients::default()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let accept = thread::Builder::new()
            .name(format!("pub_sub-accept-{}", local_addr.port()))
            .spawn({
                let clients = clients.clone();
                let shutdown = shutdown.clone();
                move || accept_loop(listener, clients, shutdown, config)
            })
            .context("failed to spawn accept thread")?;

        Ok(Publisher {
            clients,
            shutdown,
            compression: config.compression,
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
            let Clients {
                connected,
                spare,
                compressed,
            } = &mut *clients;
            // The message is compressed and framed once, however many
            // subscribers share it, and not at all if none do. Framing it here
            // rather than on each sender thread is what lets a thread write it
            // in one call, so that its length prefix shares a packet with its
            // payload.
            if connected.is_empty() {
                return;
            }
            let payload = match self.compression {
                Compression::Plain => data,
                Compression::Lz4 { .. } => match compress::compress_into(compressed, data) {
                    Ok(()) => compressed.as_slice(),
                    Err(error) => {
                        error!("Dropping a message: {error:#}");
                        return;
                    }
                },
            };
            let msg = match frame_into(spare.take(), payload) {
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
                if let Some(skips) = client.mailbox.post(Arc::clone(&msg), now) {
                    skip_reports.push((client.peer, skips));
                }
            }
            *spare = Some(msg);
        }
        for (peer, skips) in skip_reports {
            let Skips { count, over } = skips;
            if skips.is_concerning() {
                warn!("Subscriber {peer} is behind: skipped {count} messages in {over:?}.");
            } else {
                info!("Subscriber {peer} is behind: skipped {count} messages in {over:?}.");
            }
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
    ///
    /// The accept thread is not waiting on anything and needs nothing to
    /// release it: it comes round every [`Config::poll_interval`] and sees the
    /// flag, so that is the longest this waits for it.
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
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

fn accept_loop(
    listener: TcpListener,
    clients: Arc<Mutex<Clients>>,
    shutdown: Arc<AtomicBool>,
    config: Config,
) {
    // Asking whether anyone has connected, rather than waiting until someone
    // does, is what lets this loop reap between connections and notice a
    // publisher on its way out without being sent anything to wake it.
    if let Err(e) = listener.set_nonblocking(true) {
        // Every other socket option here warns and carries on, and this one
        // has more reason to: a loop that gives up accepts nobody, which is a
        // show with no clients, while a loop that waits on the listener still
        // accepts and still reaps — just only when somebody connects.
        warn!("Failed to poll for subscribers, so none will be reaped until one connects: {e}");
    }
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) => match subscribe(stream, peer, config) {
                Ok(client) => clients.lock().unwrap().connected.push(client),
                Err(e) => warn!("Failed to subscribe a client: {e:#}"),
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(config.poll_interval);
            }
            Err(e) => {
                error!("pub_sub accept error: {e}");
                // An error that persists — a process out of file descriptors,
                // say — would otherwise spin this thread and its log as fast
                // as the two of them can go.
                thread::sleep(config.poll_interval);
            }
        }
        // Reaping belongs on this thread rather than on the publisher's: it
        // happens between messages and nothing waits on it.
        reap_departed(&clients);
    }
}

/// Start the thread that writes to a newly connected subscriber.
fn subscribe(stream: TcpStream, peer: SocketAddr, config: Config) -> Result<Client> {
    // Whether a socket accepted from a listener that does not block inherits
    // that from it is the platform's business — BSD hands it down and Linux
    // does not — and a sender thread given a socket that does not block would
    // fail its first full buffer instantly rather than waiting out the
    // deadline. Saying which is wanted costs one call and settles it.
    if let Err(e) = stream.set_nonblocking(false) {
        warn!("Failed to make subscriber {peer} block: {e}");
    }
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
    // `TCP_USER_TIMEOUT` caps that wait where it exists, which is Linux and
    // not the macOS a console publishes from. The write deadline below is the
    // portable answer and covers the same failure.
    //
    // The half a show depends on is the subscriber's own end, where a
    // publisher that stops sending is idle by definition and the probes do
    // fire.
    if let Err(e) = fail_a_silent_peer(&stream, config.keepalive) {
        warn!("Failed to set keepalive on subscriber {peer}: {e}");
    }
    // The deadline that fails a write to a subscriber which has stopped
    // accepting messages, whether it has departed or is only too slow to take
    // one. Abandoning a message part-way through costs nothing: the failure
    // ends the sender thread, the client is reaped, and the subscriber
    // reconnects onto a stream that begins at a message boundary. Nothing
    // writes to this socket again — the client keeps a handle on it only to
    // shut it down.
    if let Err(e) = stream.set_write_timeout(Some(config.write_timeout)) {
        warn!("Failed to set the write deadline on subscriber {peer}: {e}");
    }

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

/// Forget the subscribers whose sender threads have stopped.
///
/// A sender thread stops when a write to its subscriber fails, which both a
/// connection that has failed and a subscriber that has stopped accepting
/// messages bring about — the second by way of the write deadline.
///
/// So a departed subscriber is discovered only while messages are still being
/// published, and only once enough of them have gone unread to fill the
/// buffers in front of it. A publisher that goes quiet, or whose messages stop
/// reaching this point because they cannot be compressed or framed, notices
/// nothing until it publishes again; the keepalive probes fail the connection
/// eventually, because a publisher that is not writing is idle by definition,
/// and the next message published finds a client already finished.
fn reap_departed(clients: &Mutex<Clients>) {
    // Dropping a client joins its sender thread, and a thread parked in a
    // write gets there only once the socket has been shut down under it. That
    // is not work to hold the lock a publisher sends under, so the departed
    // come out of the list here and are dropped once it is released.
    let departed: Vec<Client> = {
        let mut clients = clients.lock().unwrap();
        clients
            .connected
            .extract_if(.., |client| client.is_finished())
            .collect()
    };
    drop(departed);
}

// --- Subscriber ---

/// How long a subscriber waits before its first attempt to remake a
/// connection, doubling up to [`MAX_CONNECT_BACKOFF`] while the attempts fail.
const CONNECT_BACKOFF: Duration = Duration::from_millis(100);

/// The longest a subscriber waits between attempts to remake a connection.
const MAX_CONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// A subscription's connection, and whether the subscription is still wanted.
#[derive(Default)]
struct Subscription {
    /// A second handle on the connection now in use, if there is one. Shutting
    /// it down is what fails a read parked on a publisher that is sending
    /// nothing.
    socket: Option<TcpStream>,
    /// Whether the subscription has been stopped. No further message is
    /// received, and no further connection made.
    stopped: bool,
}

/// The state a subscriber shares with whoever may stop it.
#[derive(Default)]
struct SharedSubscription {
    state: Mutex<Subscription>,
    /// Signalled when the subscription is stopped, so that a subscriber
    /// waiting to try its connection again stops waiting.
    stopped: Condvar,
}

impl SharedSubscription {
    /// Whether the subscription has been stopped.
    fn is_stopped(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopped
    }

    /// Adopt `socket` as the connection a stop releases, yielding whether the
    /// subscription still wants one.
    ///
    /// A subscription stopped while the connection was being made adopts
    /// nothing, so a socket cannot be installed behind a stop and read from
    /// afterwards.
    fn adopt(&self, socket: TcpStream) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.stopped {
            return false;
        }
        state.socket = Some(socket);
        true
    }

    /// Wait up to `backoff` for the subscription to be stopped, yielding
    /// whether it is still running.
    fn wait(&self, backoff: Duration) -> bool {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, _) = self
            .stopped
            .wait_timeout_while(state, backoff, |state| !state.stopped)
            .unwrap_or_else(PoisonError::into_inner);
        !state.stopped
    }

    /// Stop the subscription, releasing a subscriber parked in a read or
    /// waiting to try its connection again.
    ///
    /// A poisoned lock is taken as it stands rather than refused, so that
    /// stopping cannot panic: it runs during teardown, where unwinding risks
    /// aborting the process.
    fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.stopped = true;
        if let Some(socket) = state.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.stopped.notify_all();
    }
}

/// Stops a subscriber that is waiting for a message, from another thread.
///
/// A subscriber blocks until a message arrives, and a publisher that has gone
/// quiet sends no last message to release it, so a subscriber handed to a
/// thread of its own cannot be taken back from it. This is how that thread is
/// released: the subscription is stopped, the read on it fails, and `recv`
/// yields nothing rather than reconnecting.
#[derive(Clone)]
pub struct SubscriberStop(Arc<SharedSubscription>);

impl SubscriberStop {
    /// Stop the subscription, so that the subscriber receives no further
    /// message and returns from the one it is waiting for. Stopping it again
    /// changes nothing.
    pub fn stop(&self) {
        self.0.stop();
    }
}

/// A TCP-based subscriber that connects to a publisher and receives every
/// message it sends. Automatically reconnects on connection loss.
pub struct Subscriber {
    host: String,
    port: u16,
    /// How this subscription's stream is carried.
    config: Config,
    stream: Option<TcpStream>,
    /// The connection, shared with the handles that can stop it.
    subscription: Arc<SharedSubscription>,
    /// The message received last, refilled by the next one to arrive.
    ///
    /// Nothing shrinks it, so one unusually large message leaves its capacity
    /// held for the life of the subscriber. The ceiling on a message length is
    /// what bounds that, and is the reason the buffer needs no bound of its
    /// own.
    buf: Vec<u8>,
    /// The message received last, expanded. Untouched on a subscription that
    /// carries its payloads as they are.
    ///
    /// It keeps its high-water mark as `buf` does, bounded by the ceiling on
    /// an expansion rather than on a message.
    plain: Vec<u8>,
}

impl Subscriber {
    /// Create a new subscriber on the stream `config` describes. Does not
    /// connect immediately — connection happens lazily on the first `recv()`
    /// call.
    pub fn new(host: impl Into<String>, port: u16, config: Config) -> Self {
        Subscriber {
            host: host.into(),
            port,
            config,
            stream: None,
            subscription: Arc::new(SharedSubscription::default()),
            buf: Vec::new(),
            plain: Vec::new(),
        }
    }

    /// A handle that stops this subscription from another thread.
    pub fn stop_handle(&self) -> SubscriberStop {
        SubscriberStop(Arc::clone(&self.subscription))
    }

    /// Block until the next message arrives. Handles reconnection internally —
    /// if the connection drops, reconnects transparently.
    ///
    /// Yields nothing once the subscription has been stopped, which is the
    /// only end to the wait a publisher cannot supply.
    ///
    /// The message stands until the next one is received, and arrives in the
    /// buffer the one before it did, so a stream of messages of a settled size
    /// costs no allocation to receive.
    pub fn recv(&mut self) -> Option<&[u8]> {
        loop {
            // Ensure we have a connection.
            if self.stream.is_none() && !self.connect() {
                return None;
            }

            // Try to read a message.
            match wire::read_msg_into(
                self.stream.as_mut().unwrap(),
                &mut self.buf,
                self.config.max_message_len,
            ) {
                Ok(()) => match self.expand() {
                    Ok(()) => break,
                    Err(e) => {
                        // Bytes that will not expand did not come from a
                        // publisher of this stream, whatever else is on the
                        // other end of the connection.
                        warn!(
                            "Subscriber received a message it could not expand ({}:{}): {e:#}",
                            self.host, self.port
                        );
                        self.stream = None;
                    }
                },
                Err(e) => {
                    // A stopped subscription is what failed the read, and is
                    // not a connection to be remade.
                    if self.subscription.is_stopped() {
                        return None;
                    }
                    // Connection lost — drop it and reconnect on next iteration.
                    warn!("Subscriber read error ({}:{}): {e}", self.host, self.port);
                    self.stream = None;
                }
            }
        }
        Some(match self.config.compression {
            Compression::Plain => &self.buf,
            Compression::Lz4 { .. } => &self.plain,
        })
    }

    /// Expand the message just received, on a subscription that carries its
    /// payloads compressed.
    fn expand(&mut self) -> Result<()> {
        match self.config.compression {
            Compression::Plain => Ok(()),
            Compression::Lz4 {
                max_decompressed_len,
            } => compress::decompress_into(&mut self.plain, &self.buf, max_decompressed_len),
        }
    }

    /// Connect to the publisher, retrying with backoff until successful, and
    /// yield whether a connection was made.
    ///
    /// A stopped subscription makes no connection and waits for none: it is
    /// the one outcome other than success.
    fn connect(&mut self) -> bool {
        let mut backoff = CONNECT_BACKOFF;

        loop {
            if self.subscription.is_stopped() {
                return false;
            }
            let addr = format!("{}:{}", self.host, self.port);
            match TcpStream::connect(&addr).and_then(|stream| {
                let socket = stream.try_clone()?;
                Ok((stream, socket))
            }) {
                Ok((stream, socket)) => {
                    // Without this, a publisher that goes away without closing
                    // leaves this subscriber blocked in a read that no message
                    // and no error will ever end.
                    if let Err(e) = fail_a_silent_peer(&stream, self.config.keepalive) {
                        warn!("Failed to set keepalive on the connection to {addr}: {e}");
                    }
                    // A connection that cannot be released is one to park a
                    // read on only to be stuck there, so the second handle on
                    // it comes before it is used.
                    if !self.subscription.adopt(socket) {
                        return false;
                    }
                    log::debug!("Subscriber connected to {addr}");
                    self.stream = Some(stream);
                    return true;
                }
                Err(e) => {
                    warn!("Subscriber connect to {addr} failed: {e}. Retrying in {backoff:?}.");
                    if !self.subscription.wait(backoff) {
                        return false;
                    }
                    backoff = (backoff * 2).min(MAX_CONNECT_BACKOFF);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    /// A limit no message in a test reaches, for the tests that are about
    /// something other than the limit.
    const TEST_MAX_MESSAGE_LEN: usize = 4 * 1024 * 1024;

    /// A stream carrying its payloads as they are, of the size a test sends.
    ///
    /// A stalled subscriber is how several tests hold a sender thread still,
    /// so the write deadline is long enough that no test reaches it by
    /// accident: a test about something other than reaping must not have a
    /// subscriber reaped out from under it.
    fn test_config() -> Config {
        Config {
            max_message_len: TEST_MAX_MESSAGE_LEN,
            write_timeout: Duration::from_secs(600),
            ..Default::default()
        }
    }

    /// The same stream, carrying its payloads compressed.
    fn compressed_config() -> Config {
        Config {
            compression: Compression::Lz4 {
                max_decompressed_len: TEST_MAX_MESSAGE_LEN,
            },
            ..test_config()
        }
    }

    /// A publisher carrying the stream `config` describes, and the port it is
    /// reached on.
    fn publisher_with(config: Config) -> (Publisher, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let publisher = Publisher::new(listener, config).unwrap();
        // Give accept thread time to start.
        thread::sleep(Duration::from_millis(50));
        (publisher, port)
    }

    fn test_publisher() -> (Publisher, u16) {
        publisher_with(test_config())
    }

    #[test]
    fn every_subscriber_receives_every_message() {
        let (publisher, port) = test_publisher();

        // Spawn two subscribers in threads since both need to recv().
        let handle1 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, test_config());
            sub.recv().unwrap().to_vec()
        });

        // Second subscriber in another thread.
        let port2 = port;
        let handle2 = thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port2, test_config());
            sub.recv().unwrap().to_vec()
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
            let mut sub = Subscriber::new("127.0.0.1", port, test_config());
            loop {
                let msg = sub.recv().unwrap().to_vec();
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

    /// A settled stream of messages is received into one allocation, whatever
    /// the sizes within it. Every message arrives whole, however large it is.
    ///
    /// Each message is published only after the one before it has been
    /// received, so a subscriber that is slow misses nothing here.
    #[test]
    fn a_settled_stream_of_messages_is_received_into_one_allocation() {
        /// A message of the size the transport carries all day.
        const SETTLED_LEN: usize = 2_700;
        /// A message short enough to fit a buffer sized for the settled one
        /// with room to spare, so that receiving it into a buffer of its own
        /// would show up as a smaller capacity.
        const SHORT_LEN: usize = 300;
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

        let mut sub = Subscriber::new("127.0.0.1", port, test_config());
        sub.connect();
        // Let the publisher's accept thread take up the subscription.
        thread::sleep(Duration::from_millis(200));

        let mut receive = |len: usize| -> usize {
            requests.send(len).unwrap();
            let received = sub.recv().unwrap();
            assert_eq!(received.len(), len);
            assert!(
                received.iter().all(|&b| b == FILL),
                "a {len}-byte message arrived holding something other than what was published"
            );
            sub.buf.capacity()
        };

        let settled = receive(SETTLED_LEN);
        assert!(
            settled >= SETTLED_LEN,
            "a message was received into a buffer too small to hold it"
        );
        for _ in 0..5 {
            assert_eq!(
                receive(SHORT_LEN),
                settled,
                "a message shorter than the one before it was received into a buffer of its own"
            );
            assert_eq!(
                receive(SETTLED_LEN),
                settled,
                "a settled stream of messages reallocated"
            );
        }
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
    ///
    /// A subscriber that stops accepting messages altogether is dropped in
    /// the end, when a write to it runs past the deadline; the deadline here
    /// is far beyond anything the test reaches, so what is measured is the
    /// cost of missing messages alone.
    #[test]
    fn a_subscriber_that_stops_reading_costs_only_itself() {
        let (publisher, port) = test_publisher();

        // Connect the subscriber that stalls first, so that a publisher
        // writing to its subscribers in turn reaches it before the other.
        let mut stalled = stalled_subscriber(port);

        let (received, receipts) = channel();
        thread::spawn(move || {
            let mut sub = Subscriber::new("127.0.0.1", port, test_config());
            loop {
                let msg = sub.recv().unwrap().to_vec();
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

    /// How long a departed subscriber gets to be noticed before the publisher
    /// is taken to be keeping it forever.
    const REAP_LIMIT: Duration = Duration::from_secs(10);

    /// A subscriber that stops accepting what it is sent is dropped, and not
    /// before the write deadline has run.
    ///
    /// A subscriber that vanishes without closing — an unplugged client, a
    /// switch that loses power — leaves behind a socket the publisher can go
    /// on writing into until its buffers fill, and a sender thread that then
    /// parks in a write with nothing to end it. Nothing about the connection
    /// says the subscriber is gone. A publisher that does not notice keeps a
    /// client, a thread and a copy of every message published for a peer that
    /// is never coming back.
    ///
    /// A peer stalled on the loopback is the hard case rather than a weaker
    /// stand-in for an unplugged one: it acknowledges everything it is sent,
    /// so no timeout the operating system keeps will ever fail the
    /// connection. Only a deadline the publisher sets itself can.
    ///
    /// Reaping removes a client whose sender thread has stopped, and a peer
    /// that closes stops one too — so an empty list on its own does not say
    /// which of the two emptied it. What says so is that the deadline had to
    /// run first, and that the peer's connection was still carrying the
    /// stream when the publisher gave up on it.
    #[test]
    fn a_subscriber_that_stops_accepting_what_it_is_sent_is_dropped() {
        /// Long enough to outlast a healthy write, short enough to keep the
        /// test quick.
        const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

        let (publisher, port) = publisher_with(Config {
            write_timeout: WRITE_TIMEOUT,
            // Reaping runs at this cadence, so it sits well inside the
            // deadline: otherwise the time a subscriber takes to disappear
            // would be this rather than the deadline, and would say nothing
            // about which of them dropped it.
            poll_interval: Duration::from_millis(100),
            ..test_config()
        });
        // Held open for the duration, so that the subscriber is stalled rather
        // than closed and its connection stays healthy throughout.
        let mut stalled = stalled_subscriber(port);
        thread::sleep(Duration::from_millis(300));

        // Published from inside the wait rather than in a burst before it: how
        // many messages it takes to fill a subscriber's buffers is the
        // operating system's business, and a test that stops publishing while
        // there is still room in them waits on a write that is not blocked.
        let bulk = vec![0xABu8; LARGE_MESSAGE];
        let started = Instant::now();
        while started.elapsed() < REAP_LIMIT {
            publisher.send(&bulk);
            if !publisher.clients.lock().unwrap().connected.is_empty() {
                continue;
            }
            let elapsed = started.elapsed();
            assert!(
                elapsed >= WRITE_TIMEOUT,
                "a subscriber was dropped after {elapsed:?}, too soon for a write to it to have \
                 run past the {WRITE_TIMEOUT:?} deadline: its connection failed rather than \
                 stalling"
            );
            let mut byte = [0u8; 1];
            let read = stalled
                .read(&mut byte)
                .expect("the stalled subscriber's connection had failed rather than stalling");
            assert_eq!(
                read, 1,
                "the stalled subscriber's connection had been closed rather than stalling"
            );
            return;
        }
        // Nothing was reaped inside the wait. Which of the three things that
        // could mean is worth knowing before guessing, and the answer is only
        // available here, so the failure states it rather than the fact of it.
        //
        // Whether the sender thread finished says whether the write deadline
        // did its job: a client that is finished had its write fail on
        // schedule and only reaping is missing. Whether the list drains once
        // this thread stops spinning says whether the accept thread was being
        // starved of the lock it needs. Whether it drains only after a
        // connection arrives says the accept loop was parked in `accept`,
        // which happens where a receive timeout does not bound one.
        let census = |when: &str| {
            let clients = publisher.clients.lock().unwrap();
            let finished = clients.connected.iter().filter(|c| c.is_finished()).count();
            format!(
                "{when}: {} connected, {finished} finished",
                clients.connected.len()
            )
        };
        let inside_the_wait = census("inside the wait");
        thread::sleep(Duration::from_millis(500));
        let after_the_spin = census("500ms after the spin stopped");
        let poke = TcpStream::connect(("127.0.0.1", port));
        thread::sleep(Duration::from_millis(500));
        let after_a_connection = census("500ms after a connection arrived");
        panic!(
            "a subscriber that accepted nothing it was sent was still subscribed after \
             {REAP_LIMIT:?}\n  {inside_the_wait}\n  {after_the_spin}\n  {after_a_connection}\n  \
             (the poke connected: {})\n  a finished client that outlives the spin but not a \
             connection means the accept loop was parked in accept; one that goes as soon as the \
             spin stops means it was starved of the clients lock; one that is never finished \
             means the write deadline never failed a write.",
            poke.is_ok()
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

    /// A compressed stream carries every message whole, whatever size it is
    /// and whatever the buffers held before it.
    ///
    /// A message expands into the buffer the one before it expanded into, so a
    /// message shorter than its predecessor leaves some of that one's bytes
    /// behind. A subscriber has to be held to what it expanded rather than to
    /// what its buffer holds.
    #[test]
    fn a_compressed_stream_carries_every_message_whole() {
        let (publisher, port) = publisher_with(compressed_config());

        let (requests, to_publish) = channel::<Vec<u8>>();
        thread::spawn(move || {
            while let Ok(msg) = to_publish.recv() {
                publisher.send(&msg);
            }
        });

        let mut sub = Subscriber::new("127.0.0.1", port, compressed_config());
        sub.connect();
        // Let the publisher's accept thread take up the subscription.
        thread::sleep(Duration::from_millis(200));

        // Long, then short, then long again, so that each message expands into
        // a buffer both larger and smaller than the one it needs.
        for len in [64_000, 40, 64_000, 0] {
            let msg: Vec<u8> = (0..len).map(|i| (i % 7) as u8).collect();
            requests.send(msg.clone()).unwrap();
            assert_eq!(
                sub.recv().unwrap(),
                msg,
                "a {len}-byte message did not arrive whole"
            );
        }
    }

    /// How long a released subscriber gets to return before it is taken to be
    /// still parked.
    const RELEASE_LIMIT: Duration = Duration::from_secs(10);

    /// Receive on `subscriber` from a thread of its own, reporting what the
    /// receive yielded.
    fn receive_on(mut subscriber: Subscriber) -> Receiver<Option<Vec<u8>>> {
        let (received, receipts) = channel();
        thread::spawn(move || {
            let _ = received.send(subscriber.recv().map(<[u8]>::to_vec));
        });
        receipts
    }

    /// A port nothing is listening on.
    fn unused_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    /// Stopping a subscription releases a subscriber that is waiting, whether
    /// it waits for a message or for a publisher to connect to.
    ///
    /// Both waits end only from outside: a publisher that has gone quiet sends
    /// no last message, and a publisher that is not running accepts no
    /// connection, so a subscriber given a thread of its own could not
    /// otherwise be taken back from it.
    #[test]
    fn stopping_a_subscription_releases_the_subscriber() {
        // Parked in a read: connected to a publisher that publishes nothing.
        let (publisher, port) = test_publisher();
        let subscriber = Subscriber::new("127.0.0.1", port, test_config());
        let stop = subscriber.stop_handle();
        let receipts = receive_on(subscriber);
        // Long enough to have connected and be waiting for a message.
        thread::sleep(Duration::from_millis(300));
        stop.stop();
        assert!(
            matches!(receipts.recv_timeout(RELEASE_LIMIT), Ok(None)),
            "a subscriber waiting for a message was not released by stopping it"
        );
        drop(publisher);

        // Waiting to connect: no publisher to connect to at all.
        let subscriber = Subscriber::new("127.0.0.1", unused_port(), test_config());
        let stop = subscriber.stop_handle();
        let receipts = receive_on(subscriber);
        // Long enough for the first attempt to have failed and the wait
        // before the second to have begun.
        thread::sleep(Duration::from_millis(300));
        stop.stop();
        assert!(
            matches!(receipts.recv_timeout(RELEASE_LIMIT), Ok(None)),
            "a subscriber waiting to connect was not released by stopping it"
        );
    }

    /// A subscription stopped before it is used receives nothing.
    #[test]
    fn a_subscription_stopped_first_never_connects() {
        let (_publisher, port) = test_publisher();
        let mut subscriber = Subscriber::new("127.0.0.1", port, test_config());
        subscriber.stop_handle().stop();
        assert!(subscriber.recv().is_none());
    }

    /// A run of skipped messages is judged by the rate it was missed at
    /// rather than by how many were missed.
    ///
    /// The count alone means nothing without the stretch of time it covers:
    /// the same run reported over a period ten times as long is a tenth of the
    /// fault, and a threshold read off the count would move the moment the
    /// reporting period did.
    #[test]
    fn skips_are_judged_by_their_rate_rather_than_their_count() {
        let skips = |count: u64, secs: f64| Skips {
            count,
            over: Duration::from_secs_f64(secs),
        };
        assert!(!skips(4, 1.0).is_concerning());
        assert!(skips(6, 1.0).is_concerning());

        // The same two rates, reported over ten times the period.
        assert!(!skips(40, 10.0).is_concerning());
        assert!(skips(60, 10.0).is_concerning());

        // A run covering no time at all has no rate to take, and is treated as
        // the fault rather than as an infinity.
        assert!(skips(1, 0.0).is_concerning());
    }

    /// A burst of missed messages is reported at the rate of the burst, not
    /// at a rate diluted by however long the subscriber was healthy before it.
    ///
    /// A subscriber that misses one message and then behaves for an hour, and
    /// one that misses hundreds in a second, are the same count over wildly
    /// different stretches of time. Counting from the last report rather than
    /// from the start of the run being reported puts the healthy hour in the
    /// denominator, and a fault that should be warned about is reported as
    /// though it were the ordinary loss this transport is built to absorb.
    #[test]
    fn a_burst_of_missed_messages_is_reported_at_the_rate_of_the_burst() {
        /// The gap between messages on a stream of a few hundred per second.
        const GAP: f64 = 0.004;

        let (mailbox, _shared) = Mailbox::new();
        let start = Instant::now();
        let at = |secs: f64| start + Duration::from_secs_f64(secs);
        let msg = || Arc::new(vec![0u8]);

        // One message missed, then a subscriber that goes on taking them.
        assert!(mailbox.post(msg(), at(0.0)).is_none());
        assert!(mailbox.post(msg(), at(GAP)).is_none());
        mailbox.take();

        // The one it missed is reported on its own once the period is up, at a
        // rate that says it is a straggler.
        let straggler = mailbox
            .post(msg(), at(1.5))
            .expect("a missed message went unreported past the reporting period");
        assert_eq!(straggler.count, 1);
        assert!(
            !straggler.is_concerning(),
            "one missed message was reported as a subscriber that has stopped keeping up"
        );
        mailbox.take();

        // An hour of health, and then a burst: every message missed for a
        // second and a half.
        let burst_at = |i: u32| at(3600.0 + f64::from(i) * GAP);
        assert!(mailbox.post(msg(), burst_at(0)).is_none());
        let reports: Vec<Skips> = (1..375)
            .filter_map(|i| mailbox.post(msg(), burst_at(i)))
            .collect();
        let [burst] = reports.as_slice() else {
            panic!(
                "a burst of 374 missed messages was reported {} times",
                reports.len()
            )
        };
        assert!(
            burst.count > 240,
            "a burst of 374 missed messages was reported as {} of them",
            burst.count
        );
        assert!(
            burst.is_concerning(),
            "{} messages missed in {:?} was reported as the ordinary loss of a subscriber that \
             is keeping up",
            burst.count,
            burst.over
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
