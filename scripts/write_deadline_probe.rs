//! Does `SO_SNDTIMEO` fail a write to a peer that has stopped reading?
//!
//! Build and run with no toolchain setup beyond rustc:
//!
//! ```
//! rustc -O scripts/write_deadline_probe.rs -o /tmp/write_deadline_probe
//! /tmp/write_deadline_probe
//! ```
//!
//! It takes under a minute and needs no network — everything is loopback.
//!
//! Three things can go wrong with a write deadline, and they want different
//! fixes, so each is asked separately:
//!
//! 1. the deadline is refused or does not stick;
//! 2. the deadline sticks but a blocked write runs past it anyway;
//! 3. the deadline fires, but as a short write rather than an error, and
//!    `write_all` retries it forever instead of giving up.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// The deadline under test.
const DEADLINE: Duration = Duration::from_millis(500);

/// How long a probe runs before the thing it is waiting for is taken not to
/// happen. Far longer than the deadline, so that a slow one still shows up as
/// a deadline rather than as this.
const PATIENCE: Duration = Duration::from_secs(20);

/// One chunk of a raw write, small enough that many go out before the buffers
/// fill and the first one blocks.
const CHUNK: usize = 64 * 1024;

/// One message, the size the publisher this is about writes in a single
/// `write_all`.
const MESSAGE: usize = 1024 * 1024;

/// A connected pair whose peer end never reads a byte, which is what a
/// departed subscriber looks like to the end still writing.
fn to_a_peer_that_never_reads() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let peer = TcpStream::connect(addr).expect("connect");
    let (writing_end, _) = listener.accept().expect("accept");
    (writing_end, peer)
}

/// Ask for the deadline and report whether it was taken, which is the first
/// way this can fail: the code under test only warns when it is refused.
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

/// Write chunks until one fails, reporting how much went out before a write
/// first blocked and what ended the blocked one.
fn raw_writes() {
    println!("\n[1] write() under a deadline, 64 KiB at a time");
    let (mut writing_end, _peer) = to_a_peer_that_never_reads();
    set_deadline(&writing_end);

    let chunk = vec![0xABu8; CHUNK];
    let mut sent: u64 = 0;
    let mut first_block: Option<(u64, Duration)> = None;
    let started = Instant::now();

    loop {
        let began = Instant::now();
        let outcome = writing_end.write(&chunk);
        let blocked = began.elapsed();
        // The first write that waits at all is the one that found the buffers
        // full: everything before it was absorbed without waiting.
        if blocked > Duration::from_millis(50) && first_block.is_none() {
            first_block = Some((sent, blocked));
            println!("  buffers filled after {sent} bytes; that write waited {blocked:?}");
        }
        match outcome {
            Ok(0) => {
                println!("  write returned Ok(0) after {sent} bytes — treated as WriteZero");
                return;
            }
            Ok(n) => {
                sent += n as u64;
                if blocked >= DEADLINE && n < CHUNK {
                    println!(
                        "  SHORT WRITE PAST THE DEADLINE: Ok({n}) of {CHUNK} after waiting \
                         {blocked:?}"
                    );
                    println!("  → the deadline fires as a short write, not an error");
                    return;
                }
            }
            Err(e) => {
                println!("  write FAILED after waiting {blocked:?}, {sent} bytes sent");
                println!(
                    "  → kind={:?} raw_os_error={:?} — {e}",
                    e.kind(),
                    e.raw_os_error()
                );
                return;
            }
        }
        if started.elapsed() > PATIENCE {
            println!("  NO FAILURE: {sent} bytes written in {PATIENCE:?}");
            match first_block {
                Some((at, waited)) => println!(
                    "  → a write blocked (after {at} bytes, waiting {waited:?}) but the deadline \
                     never failed one"
                ),
                None => println!(
                    "  → no write ever blocked: the buffers took {sent} bytes without filling"
                ),
            }
            return;
        }
    }
}

/// Write whole messages the way the publisher does, which is the case that
/// matters: `write_all` retries a short write, so a deadline that fires as one
/// is a deadline `write_all` swallows.
fn whole_messages() {
    println!("\n[2] write_all() of a 1 MiB message, which is what the publisher does");
    let (mut writing_end, _peer) = to_a_peer_that_never_reads();
    set_deadline(&writing_end);

    let message = vec![0xABu8; MESSAGE];
    let started = Instant::now();
    let mut written: u32 = 0;

    loop {
        match writing_end.write_all(&message) {
            Ok(()) => written += 1,
            Err(e) => {
                println!(
                    "  write_all FAILED on message {} after {:?}",
                    written + 1,
                    started.elapsed()
                );
                println!(
                    "  → kind={:?} raw_os_error={:?} — {e}",
                    e.kind(),
                    e.raw_os_error()
                );
                return;
            }
        }
        if started.elapsed() > PATIENCE {
            println!(
                "  NO FAILURE: {written} messages ({} MiB) written in {PATIENCE:?}",
                u64::from(written) * MESSAGE as u64 / (1024 * 1024)
            );
            println!("  → write_all never gave up on a peer that never read a byte");
            return;
        }
    }
}

/// Write a whole message on a non-blocking socket against one deadline for
/// the message rather than one per write, which is the alternative to
/// `SO_SNDTIMEO` and rests on nothing a platform decides.
fn non_blocking_with_a_deadline() {
    println!("\n[3] the alternative: non-blocking, one deadline for the whole message");
    let (mut writing_end, _peer) = to_a_peer_that_never_reads();
    match writing_end.set_nonblocking(true) {
        Ok(()) => println!("  set_nonblocking(true): ok"),
        Err(e) => println!("  set_nonblocking(true): REFUSED — {e}"),
    }

    let message = vec![0xABu8; MESSAGE];
    let started = Instant::now();
    let mut written: u32 = 0;
    let mut sent_of_message = 0;
    // The deadline is against progress on this message, so it is reset by a
    // write that moves and not by one that merely returns.
    let mut progressed_at = Instant::now();

    loop {
        match writing_end.write(&message[sent_of_message..]) {
            Ok(0) => {
                println!("  write returned Ok(0) after {} messages", written);
                return;
            }
            Ok(n) => {
                sent_of_message += n;
                progressed_at = Instant::now();
                if sent_of_message == MESSAGE {
                    written += 1;
                    sent_of_message = 0;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if progressed_at.elapsed() >= DEADLINE {
                    println!(
                        "  GAVE UP after {:?}, on message {}, {sent_of_message} bytes into it",
                        started.elapsed(),
                        written + 1
                    );
                    println!("  → the deadline bounded the whole message, as its name says");
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                println!(
                    "  write FAILED after {:?} — kind={:?}",
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

fn main() {
    println!("platform: {}", std::env::consts::OS);
    println!("deadline under test: {DEADLINE:?}");
    raw_writes();
    whole_messages();
    non_blocking_with_a_deadline();
    println!(
        "\nWanted: [2] fails, which is what the shipped code depends on.\n\
         [3] is the fallback, and should give up at about the deadline."
    );
}
