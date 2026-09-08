//! An artnet node served by this client alongside the show.
use anyhow::{Context as _, Result, bail};
use log::{info, warn};
use rust_dmx::{ArtnetNode, ArtnetNodeConfig, PortAddress};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

/// The artnet universe every client's node serves.
///
/// A controller reaches a node at its own address, carrying the universe in
/// the packet, so nodes all serving this one are still distinct outputs.
const UNIVERSE: u8 = 0;

/// An artnet node this client serves alongside the show.
///
/// The node forwards the universe it serves to a DMX interface attached to
/// this machine, so a machine with a spare USB port earns its keep as a
/// lighting node. Serving runs on a thread of its own, which lasts as long as
/// the service that owns it.
pub struct ArtnetNodeService {
    /// Cleared to bring the serving thread home.
    serving: Arc<AtomicBool>,
    /// The thread serving the node. Empty once it has been taken out to be
    /// joined.
    service: Option<JoinHandle<()>>,
}

impl ArtnetNodeService {
    /// Serve the first DMX interface attached to this machine as an artnet node.
    ///
    /// Fails when there is no interface to serve, so that a client asked for a
    /// node it cannot provide does not start at all: a missing interface is
    /// known when the client is configured rather than when the lights are
    /// wanted.
    pub fn new() -> Result<Self> {
        // Chosen once. An interface unplugged later is reopened by the port
        // itself on the next write.
        let Some(port) = rust_dmx::first_available_port()? else {
            bail!("an artnet node was requested but no DMX port is attached");
        };
        // The name this machine is discovered under, so a node and the client
        // serving it answer to the same thing.
        let machine = zero_configure::bare::machine_hostname();
        info!("Serving {port} as artnet node {machine:?} on universe {UNIVERSE}.");
        let mut node = ArtnetNode::new(
            port,
            ArtnetNodeConfig {
                port_address: PortAddress::from(UNIVERSE),
                long_name: format!("{machine} (tunnels render client)"),
                short_name: machine,
            },
        )?;
        // A controller that polled before this node existed would not otherwise
        // learn about it until it polled again.
        if let Err(err) = node.announce() {
            warn!("Could not announce the artnet node: {err:#}.");
        }
        let serving = Arc::new(AtomicBool::new(true));
        let service = thread::Builder::new()
            .name("artnet_node".to_string())
            .spawn({
                let serving = serving.clone();
                move || node.run(|| serving.load(Ordering::Relaxed))
            })
            .context("failed to spawn the artnet node thread")?;
        Ok(Self {
            serving,
            service: Some(service),
        })
    }
}

impl Drop for ArtnetNodeService {
    /// Stop the serving thread and wait for it.
    ///
    /// The thread waits for artnet packets with a timeout of its own, so it
    /// comes home within one of those waits whether or not a controller is
    /// sending to it.
    fn drop(&mut self) {
        self.serving.store(false, Ordering::Relaxed);
        if let Some(service) = self.service.take() {
            let _ = service.join();
        }
    }
}
