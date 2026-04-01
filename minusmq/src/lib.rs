//! minusmq — minimal TCP messaging for tunnels.
//!
//! Two patterns:
//! - `req_rep`: one-shot request-response over TCP
//! - `pub_sub`: persistent publish-subscribe with channel-based filtering

pub mod pub_sub;
pub mod req_rep;

mod wire;
