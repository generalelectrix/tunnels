//! minusmq — minimal TCP messaging for tunnels.
//!
//! Two patterns:
//! - `req_rep`: one-shot request-response over TCP
//! - `pub_sub`: persistent publish-subscribe, every subscriber receiving every message
//!
//! A publish-subscribe stream may carry its payloads compressed, which both of
//! its ends are configured for rather than told on the wire.

pub mod compress;
pub mod pub_sub;
pub mod req_rep;

mod wire;
