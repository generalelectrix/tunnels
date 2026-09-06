//! Protocol types for the tunnel-bootstrap binary push system.

use serde::{Deserialize, Serialize};

/// The longest a serialized push request is allowed to be.
///
/// A request is an executable and the arguments to launch it with. A release
/// build of the render client measures a few megabytes, but the build worth
/// sizing for is the debug one, which carries its debug information and
/// measures a hundred and thirty; a bootstrapper that could not take one would
/// refuse exactly the push a person is most likely to be making. The ceiling
/// is a shade under twice that, and still small enough that a length prefix
/// from a client that is confused or hostile fails the exchange rather than
/// reserving up to four gigabytes to match its claim.
pub const MAX_REQUEST_LEN: usize = 256 * 1024 * 1024;

/// The longest a serialized push response is allowed to be.
///
/// A response is one status line — a success message quoting the child's first
/// words, or the reason the push failed — so it runs to a few hundred bytes.
/// The ceiling leaves room for a bootstrapper to report something long-winded
/// while still refusing a length prefix that could only come from one that is
/// confused or hostile. The binary travels the other way and is bounded
/// separately: a push is large in one direction only.
pub const MAX_RESPONSE_LEN: usize = 64 * 1024;

/// Payload for a binary push.
#[derive(Serialize, Deserialize)]
pub struct PushBinaryRequest {
    pub sha256: [u8; 32],
    pub payload: Vec<u8>,
    /// Args to pass when launching the binary (e.g. `["monitor"]`).
    pub run_args: Vec<String>,
    /// Data to pipe into the child's stdin after launch (e.g. serialized config).
    pub stdin_payload: Vec<u8>,
}

/// Response from the bootstrapper: Ok(message) on success, Err(reason) on failure.
pub type PushBinaryResponse = Result<String, String>;
