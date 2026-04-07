# Tunnels

Tunnels is a live performance system for projecting immersive tunnels of light. It consists of a control server, render clients, and shared libraries — all running during a show in front of an audience.

## No panics during a show

All components of this system run live. A crash stops the show. Code running after initialization must not panic. If an operation can fail, log the error and recover gracefully — skip a frame, skip a shape, drop a message. Use `.unwrap()` and `.expect()` only during startup initialization where there is no meaningful recovery (e.g. GPU device creation, window creation, config parsing).

Mutex `.lock().unwrap()` is acceptable when there is no recovery path from a poisoned mutex.

## Pre-commit hook

A pre-commit hook that runs `cargo fmt` is checked into `.githooks/`. After cloning, enable it with:

```
git config core.hooksPath .githooks
```

## Prefer vendored C dependencies

When a crate wraps a C library (OpenSSL, SDL2, libssh2, etc.), prefer the `vendored` or `bundled` feature so the library is built from source. This keeps builds self-contained and avoids cross-compilation failures from missing system libraries. Use judgment — if vendoring introduces significant downsides (massive build times, licensing issues, etc.), discuss the tradeoff first.

## Audio subsystem

See `tunnels/src/audio/CLAUDE.md` for detailed architecture. Key points:

- The audio callback is a real-time context — no allocations, no locks, no panics.
- Parameters are passed via AtomicF32 fields in an Arc-shared struct, not through message channels. The audio thread polls for changes at the start of each buffer.
- The production GUI communicates via ControlMessage → show loop → atomic writes. Only the dev tool (audio_vis) writes atomics directly.
- The render loop runs at 240fps. The audio buffer is ~1ms. The fast envelope follower's 4ms release matches the render frame budget.

## GUI architecture

- The console GUI sends `MetaCommand`s and `ControlMessage`s through a channel. The show loop processes them and emits `StateChange`s back to all listeners.
- GUI reads state from `SharedGuiState` snapshots (via `arc-swap`), not from internal atomics. Unidirectional flow: GUI → commands → show → state snapshots → GUI.
- Shared GUI components live in `gui_common/`. Panel pattern: state struct + render struct with `GuiContext` for sending commands.

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `tunnels` | Main library: show loop, audio, animation, clocks, MIDI, rendering |
| `tunnels_lib` | Shared utilities: number types, colors, smoothing, prompts |
| `console` | GUI binary (eframe/egui): show configuration, MIDI, audio, animation viz |
| `audio_vis` | Dev tool: standalone audio visualization for DSP development |
| `tunnelclient` | Render client |
| `tunnel-bootstrap` | Client bootstrapping |
| `gui_common` | Shared GUI components (status colors, modals, panels) |
| `stage_theme` | Dark egui theme for stage environments |
| `midi_harness` | MIDI device management |
| `zero_configure` | DNSSD service discovery |
| `minusmq` | Messaging |
| `bonsoir` | Bonjour/DNSSD wrapper |
| `bootstrap-deploy` | Deployment tool |
