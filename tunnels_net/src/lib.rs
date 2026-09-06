//! The network services a show is made of.
//!
//! A service here owns one protocol at both ends: the port it runs on, the
//! payload it carries, the ceiling on a message of it, and the halves that
//! publish and consume it. Nothing about a protocol is written at a call site,
//! so the two ends of it cannot drift apart.

pub mod frame_service;
