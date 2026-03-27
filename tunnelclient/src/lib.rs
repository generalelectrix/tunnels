pub mod bootstrap_controller;
pub mod draw;

/// Subcommand arg that runs the lightweight health check.
pub const ARG_SELF_TEST: &str = "self-test";
/// Subcommand arg that runs as a remotely-configurable render client.
pub const ARG_REMOTE: &str = "remote";
