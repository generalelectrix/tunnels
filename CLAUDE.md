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

The audio system lives in the `tunnels_audio` crate. Key points:

- The audio callback is a real-time context — no allocations, no locks, no panics.
- Control parameters are passed via atomic fields in `ProcessorSettings` (an `Arc`-shared struct). The audio thread polls for changes at the start of each buffer.
- The GUI reads parameter state from an `AudioSnapshot` built by `AudioInput::snapshot()` and published through `GuiState::audio_state` (a `Notified<AudioSnapshot>`). Writes to the snapshot atomically wake the GUI.
- Envelope data streams from the audio thread to the GUI via lock-free SPSC ring buffers (`EnvelopeProducer`/`EnvelopeStream` in `ring_buffer.rs`, backed by `rtrb`). On every successful device open — initial and each reconnect — the audio thread sends a fresh `EnvelopeStreams` bundle over an `mpsc` channel owned by the console's `ConfigApp`, which reattaches the envelope viewer without user intervention.
- `Processor` structures its work as an array-of-structs: a `Vec<LowpassChannel>` (one per audio channel) and a `[WaveletBand; NUM_BANDS]`. Shared bookkeeping (`OnePoleSmoother` state, `SmootherCoeff` cache, `AdaptiveNormalizer`) lives on `Processor` itself; parameter propagation to the chains runs in `maybe_update_parameters`.
- The `tunnels/src/audio/` module is a thin re-export layer plus the `ShowEmitter` adapter.
- The render loop runs at 240fps. The audio buffer is ~1ms. The fast envelope follower's 4ms release matches the render frame budget.

## GUI architecture

- The console GUI sends `MetaCommand`s and `ControlMessage`s through a channel. The show loop processes them and emits `StateChange`s back to all listeners. **Unidirectional flow is a design rule, not a suggestion**: GUI → commands → show → state snapshots → GUI. GUI code does not write directly to show-facing atomics; instead it sends a `MetaCommand` that the show handles.
- `GuiState` (in `tunnels/src/gui_state.rs`) is the show → GUI surface. Fields the GUI should repaint on use `Notified<T>` / `NotifiedAtomicBool` from `tunnels_lib::notified`, which wrap `ArcSwap<T>` / `AtomicBool` and fire a `RepaintSignal` atomically with the write. Streaming state that's already driving its own continuous repaint (e.g. `animation_state`) stays as raw `ArcSwap<T>`.
- The `RepaintSignal` is a `tunnels_lib::repaint::RepaintSignal` — `Arc<dyn Fn() + Send + Sync>`. The console wraps `egui::Context::request_repaint` inside the eframe creator closure; tests and headless callers use `noop_repaint()`.
- Shared GUI components live in `gui_common/`. Panel pattern: state struct + render struct with `GuiContext` for sending commands. Edge-triggered GUI state reported to the show (e.g. visualizer visibility) uses `gui_common::tracked::TrackedBool` to detect changes and avoid re-sending.

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `tunnels` | Main library: show loop, audio, animation, clocks, MIDI, rendering |
| `tunnels_audio` | Audio input, envelope extraction, wavelet decomposition, ring buffers |
| `tunnels_lib` | Cross-crate primitives: number types, color, smoothing, GUI repaint (`RepaintSignal`, `Notified`), transient indicator, bootstrap push protocol |
| `console` | GUI binary (eframe/egui): show configuration, MIDI, audio, animation viz |
| `tunnelclient` | Render client |
| `tunnel-bootstrap` | Client bootstrapping |
| `gui_common` | Shared GUI components (audio panel, envelope viewer, MIDI panel, scrolling plot) |
| `stage_theme` | Dark egui theme for stage environments |
| `midi_harness` | MIDI device management |
| `zero_configure` | DNSSD service discovery |
| `minusmq` | Messaging |
| `bonsoir` | Bonjour/DNSSD wrapper |
| `bootstrap-deploy` | Deployment tool |
