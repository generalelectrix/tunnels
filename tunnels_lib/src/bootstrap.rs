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
/// A response arrives behind a length prefix, and a reader that trusted the
/// prefix would allocate whatever it asked for — up to four gigabytes from one
/// that had been corrupted, or that came from something other than a
/// bootstrapper. This is what the prefix is checked against.
///
/// It leaves room far past anything a bootstrapper has to say, deliberately.
/// A response is how a failed push explains itself, and a bound tight enough
/// to truncate that explanation would withhold the report exactly when it is
/// the only thing worth having.
pub const MAX_RESPONSE_LEN: usize = 128 * 1024 * 1024;

// TODO: carry this protocol in postcard, so that the codebase speaks one
// serialization format.
//
// The peer on the other end of this exchange is a bootstrapper, which is
// installed on its own rather than pushed and so is not replaced when a show's
// binaries are rebuilt. Changing the format leaves a console unable to push to
// a machine still running the bootstrapper it has — and this is the mechanism
// reached for when something else has already gone wrong.
//
// `bootstrap-deploy` installs a fresh bootstrapper over SSH, finding machines
// by `_ssh._tcp`. Once every render machine has been refreshed, this can move
// with the rest.
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
