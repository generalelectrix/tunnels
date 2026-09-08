//! Does a write deadline fail a write to a peer that has stopped reading?
//!
//! Build and run with no toolchain setup beyond rustc:
//!
//! ```
//! rustc -O scripts/write_deadline_probe.rs -o /tmp/write_deadline_probe
//! /tmp/write_deadline_probe
//! ```
//!
//! It takes about a minute and needs no network — everything is loopback.
//!
//! `SO_SNDTIMEO` firing is not the same as a publisher noticing, because the
//! publisher writes with `write_all`, which retries a short write. So a
//! deadline that fires as a short write rather than as an error is a deadline
//! `write_all` swallows, and the sender never gives up. Each section below
//! asks one question, and the answers want different fixes.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::AsRawFd;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The socket-option numbers and the `timeval` layout differ between the two
/// platforms this runs on, and neither is worth a dependency.
#[cfg(target_os = "linux")]
mod sockopt {
    pub const SOL_SOCKET: i32 = 1;
    pub const SO_RCVTIMEO: i32 = 20;
    pub type Microseconds = i64;
}
#[cfg(target_os = "macos")]
mod sockopt {
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_RCVTIMEO: i32 = 0x1006;
    pub type Microseconds = i32;
}

#[repr(C)]
struct Timeval {
    seconds: i64,
    microseconds: sockopt::Microseconds,
}

extern "C" {
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const std::ffi::c_void, len: u32) -> i32;
}

/// The deadline under test.
const DEADLINE: Duration = Duration::from_millis(500);

/// How long a probe runs before the thing it is waiting for is taken not to
/// happen. Far longer than the deadline, so that a slow failure still shows up
/// as a deadline rather than as this.
const PATIENCE: Duration = Duration::from_secs(20);

/// The chunk sizes a raw write is tried at.
///
/// Whether a blocked write comes back as an error or as a short write may be a
/// matter of how much room was left when the deadline struck rather than of
/// the platform, and asking at one size cannot tell those apart: a small chunk
/// that does not fit at all has nothing to return short.
const CHUNKS: [usize; 3] = [4 * 1024, 64 * 1024, 1024 * 1024];

/// One message, the size the publisher this is about writes in a single
/// `write_all`.
const MESSAGE: usize = 1024 * 1024;

/// How many blocked writes are reported one by one before the rest are only
/// counted.
const WRITES_TO_SHOW: u32 = 8;

/// A connected pair whose peer end never reads a byte, which is what a
/// departed subscriber looks like to the end still writing.
///
/// The peer is returned rather than dropped: a closed peer fails a write
/// outright, which is the case these are all trying not to measure.
fn to_a_peer_that_never_reads() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let peer = TcpStream::connect(addr).expect("connect");
    let (writing_end, _) = listener.accept().expect("accept");
    (writing_end, peer)
}

/// Ask for the deadline and report whether it was taken, which is one way this
/// can fail: the code under test only warns when it is refused.
fn set_deadline(stream: &TcpStream) {
    match stream.set_write_timeout(Some(DEADLINE)) {
        Ok(()) => println!("  set_write_timeout({DEADLINE:?}): ok"),
        Err(e) => println!("  set_write_timeout({DEADLINE:?}): REFUSED — {e}"),
    }
    match stream.write_timeout() {
        Ok(Some(d)) => println!("  reads back as: {d:?}"),
        Ok(None) => println!("  reads back as: NONE — the deadline did not stick"),
        Err(e) => println!("  reads back as: error — {e}"),
    }
}

/// Report what a write returned, when it took longer than the deadline.
fn describe(blocked: Duration, outcome: &std::io::Result<usize>, asked: usize) {
    match outcome {
        Ok(n) => println!("    blocked {blocked:?}, then returned Ok({n}) of {asked} asked"),
        Err(e) => println!(
            "    blocked {blocked:?}, then failed: kind={:?} raw_os_error={:?}",
            e.kind(),
            e.raw_os_error()
        ),
    }
}

/// Write chunks until one fails, reporting how much the buffers took before a
/// write first blocked and what ended the blocked one.
fn raw_writes(chunk_len: usize) {
    println!("\n[1] write() under a deadline, {chunk_len} bytes at a time");
    let (mut writing_end, _peer) = to_a_peer_that_never_reads();
    set_deadline(&writing_end);

    let chunk = vec![0xABu8; chunk_len];
    let mut sent: u64 = 0;
    let mut writes: u32 = 0;
    let started = Instant::now();

    loop {
        let began = Instant::now();
        let outcome = writing_end.write(&chunk);
        let blocked = began.elapsed();
        writes += 1;
        match outcome {
            Ok(n) if blocked < DEADLINE => sent += n as u64,
            ref outcome => {
                println!("  buffers took {sent} bytes in {} writes first", writes - 1);
                describe(blocked, outcome, chunk_len);
                match outcome {
                    Ok(n) => println!(
                        "  → THE DEADLINE FIRES AS A SHORT WRITE ({n} bytes), not an error"
                    ),
                    Err(_) => println!("  → the deadline fires as an error, which write_all keeps"),
                }
                return;
            }
        }
        if started.elapsed() > PATIENCE {
            println!("  NO FAILURE: {sent} bytes in {writes} writes, none blocked past the");
            println!("  deadline → nothing here ever waited, so this is not about the deadline");
            return;
        }
    }
}

/// Run the loop `write_all` runs, reporting each write that blocks past the
/// deadline, because that sequence is what decides whether `write_all` ever
/// gives up.
///
/// A short write is retried, so the deadline only becomes an error once a
/// write spends the whole of it having moved nothing. A platform that hands
/// back a trickle each time never reaches that write.
fn the_loop_write_all_runs() {
    println!("\n[2] the loop write_all runs, with every blocked write reported");
    println!("    asks: does the deadline ever turn into an error, or only short writes");
    let (mut writing_end, _peer) = to_a_peer_that_never_reads();
    set_deadline(&writing_end);

    let message = vec![0xABu8; MESSAGE];
    let started = Instant::now();
    let mut sent = 0usize;
    let mut blocked_writes: u32 = 0;

    loop {
        let began = Instant::now();
        let outcome = writing_end.write(&message[sent..]);
        let blocked = began.elapsed();
        if blocked >= DEADLINE {
            blocked_writes += 1;
            if blocked_writes <= WRITES_TO_SHOW {
                describe(blocked, &outcome, MESSAGE - sent);
            }
        }
        match outcome {
            Ok(n) => {
                sent += n;
                if sent == MESSAGE {
                    sent = 0;
                }
            }
            Err(e) => {
                println!(
                    "  a write FAILED after {:?}: kind={:?} raw_os_error={:?}",
                    started.elapsed(),
                    e.kind(),
                    e.raw_os_error()
                );
                println!(
                    "  → {blocked_writes} writes blocked past the deadline before one failed; \
                     write_all gives up"
                );
                return;
            }
        }
        if started.elapsed() > PATIENCE {
            println!("  NO FAILURE in {PATIENCE:?}: {blocked_writes} writes blocked past the");
            println!("  deadline and every one of them returned bytes");
            println!("  → write_all retries forever; the deadline never becomes an error");
            return;
        }
    }
}

/// Run the real `write_all` and report whether it ever returns.
///
/// It runs on a thread of its own because the case under test is one where it
/// does not return, and a probe waiting for it inline could only hang.
fn real_write_all() {
    println!("\n[3] write_all() itself, watched from outside");
    println!("    asks: does the call the publisher makes ever come back");
    let (writing_end, peer) = to_a_peer_that_never_reads();
    set_deadline(&writing_end);

    let (done, outcome) = mpsc::channel();
    thread::spawn(move || {
        // Held for as long as the writing goes on, so the peer stays open and
        // silent rather than closing and failing the write for the wrong
        // reason.
        let _peer = peer;
        let mut writing_end = writing_end;
        let message = vec![0xABu8; MESSAGE];
        let started = Instant::now();
        let mut written: u32 = 0;
        loop {
            match writing_end.write_all(&message) {
                Ok(()) => written += 1,
                Err(e) => {
                    let _ = done.send(Err((written + 1, started.elapsed(), format!("{e:?}"))));
                    return;
                }
            }
            if started.elapsed() > PATIENCE {
                let _ = done.send(Ok(written));
                return;
            }
        }
    });

    // Longer than the thread's own patience, so that a thread which finished
    // is not reported as one that never did.
    match outcome.recv_timeout(PATIENCE + Duration::from_secs(5)) {
        Ok(Err((message_number, at, error))) => {
            println!("  write_all FAILED on message {message_number} after {at:?}: {error}");
            println!("  → the publisher would drop this subscriber");
        }
        Ok(Ok(written)) => {
            println!("  NO FAILURE: {written} messages written to a peer that never read");
            println!("  → write_all returned every time; the publisher never notices");
        }
        Err(_) => {
            println!("  NEVER RETURNED: still inside one write_all after {PATIENCE:?}");
            println!("  → the publisher's sender thread is parked there for good");
        }
    }
}

/// Write whole messages on a non-blocking socket against one deadline for the
/// message rather than one per write, which is the alternative to
/// `SO_SNDTIMEO` and rests on nothing a platform decides.
fn non_blocking_with_a_deadline() {
    println!("\n[4] the alternative: non-blocking, one deadline for the whole message");
    println!("    asks: does giving up ourselves bound what the deadline is named for");
    let (mut writing_end, _peer) = to_a_peer_that_never_reads();
    match writing_end.set_nonblocking(true) {
        Ok(()) => println!("  set_nonblocking(true): ok"),
        Err(e) => println!("  set_nonblocking(true): REFUSED — {e}"),
    }

    let message = vec![0xABu8; MESSAGE];
    let started = Instant::now();
    let mut written: u32 = 0;
    let mut sent = 0usize;
    // The deadline is against progress on this message, so a write that moves
    // resets it and a write that merely returns does not.
    let mut progressed_at = Instant::now();

    loop {
        match writing_end.write(&message[sent..]) {
            Ok(0) => {
                println!("  write returned Ok(0) after {written} messages");
                return;
            }
            Ok(n) => {
                sent += n;
                progressed_at = Instant::now();
                if sent == MESSAGE {
                    written += 1;
                    sent = 0;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if progressed_at.elapsed() >= DEADLINE {
                    println!(
                        "  GAVE UP after {:?}, on message {}, {sent} bytes into it",
                        started.elapsed(),
                        written + 1
                    );
                    println!("  → bounded by the deadline, which is what its name says");
                    return;
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                println!(
                    "  write FAILED after {:?}: kind={:?}",
                    started.elapsed(),
                    e.kind()
                );
                return;
            }
        }
        if started.elapsed() > PATIENCE {
            println!("  NO FAILURE: {written} messages written in {PATIENCE:?}");
            return;
        }
    }
}

/// How long a listener is given to come back from `accept()` with nobody
/// connecting, which is what the publisher's accept loop depends on to reap
/// between connections.
const ACCEPT_TIMEOUT: Duration = Duration::from_millis(200);

/// Does a receive timeout bound `accept()`?
///
/// The publisher sets one on its listener and reaps departed subscribers each
/// time the wait runs out. `SO_RCVTIMEO` bounds a receive, and whether an
/// `accept` counts as one is the platform's business: on a platform where it
/// does not, that thread is parked until somebody connects, and nothing is
/// ever reaped between connections however promptly a sender thread fails.
fn accept_under_a_timeout() {
    println!("\n[5] accept() under a receive timeout, with nobody connecting");
    println!("    asks: does the publisher's accept loop come round to reap on its own");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let timeout = Timeval {
        seconds: ACCEPT_TIMEOUT.as_secs() as i64,
        microseconds: ACCEPT_TIMEOUT.subsec_micros() as sockopt::Microseconds,
    };
    let applied = unsafe {
        setsockopt(
            listener.as_raw_fd(),
            sockopt::SOL_SOCKET,
            sockopt::SO_RCVTIMEO,
            &timeout as *const Timeval as *const std::ffi::c_void,
            std::mem::size_of::<Timeval>() as u32,
        )
    };
    if applied != 0 {
        println!("  SO_RCVTIMEO was REFUSED on the listener");
        return;
    }
    println!("  SO_RCVTIMEO({ACCEPT_TIMEOUT:?}) set on the listener");

    // On a thread, because the outcome under test is a call that never comes
    // back, which cannot be reported from inside it.
    let (done, outcome) = mpsc::channel();
    thread::spawn(move || {
        let began = Instant::now();
        let result = listener.accept();
        let _ = done.send((
            began.elapsed(),
            result.err().map(|e| format!("{:?}", e.kind())),
        ));
    });

    match outcome.recv_timeout(Duration::from_secs(5)) {
        Ok((waited, Some(kind))) => {
            println!("  accept() returned after {waited:?} with {kind}");
            println!("  → the wait is bounded; the accept loop reaps on its own");
        }
        Ok((waited, None)) => println!("  accept() somehow succeeded after {waited:?}"),
        Err(_) => {
            println!("  accept() NEVER RETURNED: still waiting after 5s");
            println!("  → SO_RCVTIMEO does not bound accept() here, so the accept loop");
            println!("    only comes round when somebody connects, and REAPING NEVER RUNS");
        }
    }
}

fn main() {
    println!("platform: {}", std::env::consts::OS);
    println!("deadline under test: {DEADLINE:?}");
    println!("\n[1] asks: does the deadline fire, how deep are the buffers, and does a");
    println!("blocked write come back as an error or as a short write");
    for chunk in CHUNKS {
        raw_writes(chunk);
    }
    the_loop_write_all_runs();
    real_write_all();
    non_blocking_with_a_deadline();
    accept_under_a_timeout();
    println!(
        "\n[2] and [3] are the ones that decide it: the publisher calls write_all,\n\
         so a deadline [1] reports as a short write is one [2] and [3] may never\n\
         turn into a failure. [4] is the fallback.\n\
         [5] is a separate question with the same symptom: a sender thread that\n\
         fails promptly still leaves its client on the list if nothing reaps it."
    );
}
