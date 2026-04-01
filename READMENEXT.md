# tunnels

Tunnels is a system for projecting immersive tunnels of projected light in haze,
enclosing an audience inside rich, beautiful, dynamic temporary spaces.

## Architecture

The system has four components:

- **Console** — the main GUI application (egui). Runs the control server, manages MIDI controllers, and provides an admin panel for launching and configuring render clients. This is what you interact with.
- **Render client** (`tunnelclient`) — receives tunnel state over the LAN and renders it to video via SDL2/OpenGL. Clients are launched and configured from the console's admin panel, either locally or pushed to remote machines via the bootstrapper.
- **Bootstrapper** (`tunnel-bootstrap`) — a daemon that runs on remote client machines. Receives binary pushes from the console over TCP and manages the render client process.
- **Bootstrap deploy** (`bootstrap-deploy`) — a CLI tool for initially deploying the bootstrapper to remote machines over SSH. Discovers machines via mDNS.

Service discovery between components uses mDNS/Bonjour. Data is serialized with MessagePack.

## Hardware

Minimally:

- 1x Mac, which can co-host the console and a render client.
- 1x large-format tablet (such as an iPad) running TouchOSC v1.
- 1x digital video projector, preferably with excellent contrast.
- 1x hazer, or a lot of incense.

Recommended:

- 1x Akai APC40
- 1x Behringer CMD-MM1

Nice to have:

- More projectors, with client Macs to run them.

## Controllers

Install `controller_templates/tunnels.touchosc` on a tablet running TouchOSC v1. Natively scaled for iPad. Install TouchOSC Bridge on the host computer, launch it, make sure `Enable USB Connections` is checked, and connect your tablet to the host with USB. In TouchOSC connection settings, make sure everything besides the TouchOSC Bridge connection is disabled.

Akai APC40 should work out of the box.

## Installation

Download the latest DMG from the [releases page](https://github.com/generalelectrix/tunnels/releases), open it, and drag `Tunnels.app` to Applications.

The app is unsigned — on first launch, right-click > Open (or run `xattr -cr /Applications/Tunnels.app`).

## Building from source

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install)
- [Homebrew](https://brew.sh/)
- `brew install cmake librsvg create-dmg`

For universal (Intel + Apple Silicon) builds, install both targets:

```
rustup target add x86_64-apple-darwin aarch64-apple-darwin
```

### App bundle

```
VERSION=2026.04.01-1 scripts/build-app.sh
```

This produces `dist/Tunnels.app` and `dist/Tunnels.dmg`.

### Development builds

To build and run individual components during development:

```
cargo run --release -p console       # run the console GUI
cargo run --release -p tunnelclient -- <channel> <config>  # run a client from a config file
```

See `tunnelclient/cfg/` for example client configurations.

## Setting up remote clients

1. Enable Remote Login (SSH) on the target Mac: System Settings > General > Sharing > Remote Login.
2. Run `bootstrap-deploy` from the app bundle or build it with `cargo run -p bootstrap-deploy`. It discovers machines via mDNS, deploys the bootstrapper, and registers it as a launchd service.
3. Once the bootstrapper is running, the target machine appears in the console's admin panel. Configure resolution and video channel, then push to start rendering.

## Logs

Console logs go to macOS unified logging. View them with:

```
log stream --predicate 'subsystem == "com.generalelectrix.tunnels"'
```

Or open Console.app and filter by subsystem.
