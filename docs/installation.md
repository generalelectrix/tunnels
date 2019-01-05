# Server installation and running (Python 2)

0. In a fresh virtual environment: $ python setup.py install
0. `$ ./run.sh`

# Controllers

Install `controller_templates/pytunnel.touchosc` on a tablet of your choice running TouchOSC.  Natively scaled for iPad.  Connect to a network midi session on the server via Audio Midi Setup/Core MIDI ([you may need to turn ipv6 off](https://discussions.apple.com/thread/7695767)).

Note that you may need to rename your network session until I fix [#20](https://github.com/generalelectrix/pytunnel/issues/20).  If you have funky midi devices connected to your system, be wary of [#17](https://github.com/generalelectrix/pytunnel/issues/17).

APC40 and APC20 should work out of the box.

# Building the render client/administrator (Mac)

0. Install Rust: https://www.rust-lang.org/tools/install
0. Install Homebrew: https://brew.sh/
0. `$ brew install sdl2`
0. `$ brew install zmq`
0. Inside `tunnelclient/` `$ cargo build --release`
0. Get up and make some tea or something while it compiles.

# Running the render client

To start the client as a remotely discoverable/configured service (uses dnssd/bonjour):
`$ cargo run --release remote`

To discover and administrate clients from the host:
`$ cargo run admin`

To start the client from a configuration file: from inside `tunnelclient/`,
`$ cargo run --release <virtual video channel (0 - 7)> <path to configuration file>`
See `tunnelclient/cfg/` for examples.