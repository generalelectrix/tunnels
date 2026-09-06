//! minusmq — minimal TCP messaging for tunnels.
//!
//! Two patterns:
//! - `req_rep`: one-shot request-response over TCP
//! - `pub_sub`: persistent publish-subscribe, every subscriber receiving every message
//!
//! Each pattern is configured by a struct of its own rather than by an
//! argument list, so a caller states what it cares about and takes the rest as
//! it comes. A publish-subscribe stream may carry its payloads compressed,
//! which both of its ends are configured for rather than told on the wire.

pub mod compress;
pub mod pub_sub;
pub mod req_rep;

mod wire;

/// The longest message either pattern accepts unless it is told otherwise.
///
/// A length prefix arrives before the bytes it describes, so a reader that
/// trusted one would size a buffer to whatever the prefix claimed — up to four
/// gigabytes from a prefix that had been corrupted in transit, or that came
/// from something which is not a peer of this protocol at all. The bound is
/// what a reader checks the prefix against, and nothing more.
///
/// It sits far above anything these patterns are used to carry, deliberately.
/// A prefix refused for being absurd costs one connection; a message refused
/// for having merely grown costs the thing the connection was carrying, and
/// leaves whoever is watching nothing to act on.
pub const DEFAULT_MAX_MESSAGE_LEN: usize = 128 * 1024 * 1024;
