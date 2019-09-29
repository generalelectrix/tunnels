# pytunnel

Tunnels is a system for projecting immersive tunnels of projected light in haze,
enclosing an audience inside rich, beautiful, dynamic temporary spaces.

## Architecture

The core of tunnelz is a Python control server.  The server is responsible for
interacting with midi control interfaces, storing and updating the state of the
objects encoding the tunnels, and rendering those tunnels to a compact binary
format to send over a LAN to clients.  The clients run a compact Rust program
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

## Server installation and running

In a fresh virtual environment:
0. `$ pip install cython numpy`
0. `$ python setup.py install`
0. `$ ./run.sh`

## Controllers

Install `controller_templates/pytunnel.touchosc` on a tablet of your choice running TouchOSC.  Natively scaled for iPad.  Connect to a network midi session on the server via Audio Midi Setup/Core MIDI ([you may need to turn ipv6 off](https://discussions.apple.com/thread/7695767)).

Note that you may need to rename your network session until I fix [#20](https://github.com/generalelectrix/pytunnel/issues/20).  If you have funky midi devices connected to your system, be wary of [#17](https://github.com/generalelectrix/pytunnel/issues/17).

APC40 and APC20 should work out of the box.

## Building the render client/administrator (Mac)

0. Install Rust: https://www.rust-lang.org/tools/install
0. Install Homebrew: https://brew.sh/
0. `$ brew install sdl2`
0. `$ brew install zmq`
0. `$ brew install pkgconfig` (needed for Rust to find libzmq)
0. Inside `tunnelclient/` `$ cargo build --release`
0. Get up and make some tea or something while it compiles.

## Running the render client

To start the client as a remotely discoverable/configured service (uses dnssd/bonjour):
`$ cargo run --release remote`

To discover and administrate clients from the host:
`$ cargo run admin`

To start the client from a configuration file: from inside `tunnelclient/`,
`$ cargo run --release <virtual video channel (0 - 7)> <path to configuration file>`
See `tunnelclient/cfg/` for examples.