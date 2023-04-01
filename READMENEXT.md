# tunnels

Tunnels is a system for projecting immersive tunnels of projected light in haze,
enclosing an audience inside rich, beautiful, dynamic temporary spaces.

## Architecture

The core of tunnels is a control server. The server is responsible for
interacting with midi control interfaces, storing and updating the state of the
objects encoding the tunnels, and rendering those tunnels to a compact binary
format to send over a LAN to clients. The clients run a program
that exposes them to the server via DNSSD (aka Bonjour), which then subscribe to
a virtual video stream broadcast by the server and render the feeds to video.
Interaction between the server and clients is mediated by 0MQ and msgpack.

## Hardware requirements

Minimally:

- 1x Mac, which can co-host the server and a client.
- 1x digital video projector, preferably with excellent contrast.
- 1x hazer, or a lot of incense.
- 1x large-format tablet (such as an iPad) running TouchOSC.

Recommended:

- 1x Akai APC-40

Nice to have:

- Lots more projectors, with client computers to run them.

## Controllers

Install `controller_templates/pytunnel.touchosc` on a tablet of your choice running TouchOSC. Natively scaled for iPad. Connect to a network midi session on the server via Audio Midi Setup/Core MIDI ([you may need to turn ipv6 off](https://discussions.apple.com/thread/7695767)).

APC40 and APC20 should work out of the box.

## Running the server

0. `$ cd tunnels`
1. `$ cargo run --release`

## Building the render client/administrator (Mac)

0. Install Rust: https://www.rust-lang.org/tools/install
1. Install Homebrew: https://brew.sh/
2. `$ brew install sdl2`
3. Inside `tunnelclient/` `$ cargo build --release`
4. Get up and make some tea or something while it compiles.

## Running the render client

To start the client as a remotely discoverable/configured service (uses dnssd/bonjour):
`$ cargo run --release remote`

To discover and administrate clients from the host:
`$ cargo run admin`

To start the client from a configuration file: from inside `tunnelclient/`,
`$ cargo run --release <virtual video channel (0 - 7)> <path to configuration file>`
See `tunnelclient/cfg/` for examples.
