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
- The processor's `InputMeter` (`input_meter.rs`) holds atomics for the automatic trim's gain and whether the clip indicator is lit, written once per buffer. The processor holds the only strong reference; on every successful open the reconnect thread sends a `Weak` handle to it, with the frame reader, as the stream's `InputReaders` to the show's `AudioInput`. Each show tick reads it and stores the clip LED and the trim (rounded to the 0.5 dB display step) into `GuiState::input_clip_lit` / `input_trim_db` only when they change; when a disconnect drops the stream the handle stops upgrading and both become `None` (no LED, no trim). Envelope streams go straight from the processor to the GUI; everything else the GUI shows goes through `GuiState`.
- Each input buffer is checked for a run of samples pinned at full scale on any one channel; a buffer that has one lights the clip indicator, which stays lit for 300 ms. The mono mix then passes through the automatic trim, a guard that zeroes non-finite samples, and a DC blocker.
- `Processor` runs the mono mix through a 26-band constant-Q resonator bank (`bank.rs`: two octave bands below 250 Hz, then quarter octaves to 16 kHz; complex resonators, so each band's magnitude is its envelope) and derives five roles from it (`roles.rs`), following the bank sample by sample and reporting once per buffer, so no role depends on where the music falls against the buffers: Kick and Hats are level-independent hits in the low and high end, Bass, Mid and Shimmer are the low, middle and high levels through the operator's envelope follower, smoother and an `AdaptiveNormalizer`. The shared smoother coefficient and `NormalizerParams` live on `Processor`, and parameter propagation runs in `maybe_update_parameters`.
- The same bank feeds the spectrum (`spectrum.rs`): every band followed sample by sample, whitened by a fixed corpus EQ (`CORPUS_LEVEL_DB`, measured with `examples/spectrum_corpus.rs`) and a slow adaptive tilt, then read in a 30 dB window under one motion-clocked ceiling and low-passed at 30 Hz.
- Each buffer, the processor publishes every role and band as one `AudioFrame` (`tunnels_lib::audio`, values as `UnipolarF32`) through a triple buffer (`frame_buffer.rs`); `AudioInput::frame()` reads the newest. The show owns which role it follows (`active_role`, runtime state only) and passes `AudioState { frame, active_role }` everywhere audio is used: the model, `ShowFrame` and `SharedClockData`.
- The `tunnels/src/audio/` module is a thin re-export layer plus the `ShowEmitter` adapter.
- The audio callback sets flush-to-zero on its thread (`denormals.rs`), so filters decaying on digital silence don't fall into slow subnormal arithmetic.
- The render loop runs at 240fps. The audio buffer is ~1ms. The hit roles hold each peak for 4ms, about a render frame, and fall over 80ms.
- The envelope chain is pinned by `tunnels_audio/tests/envelope_golden.rs` and `music_convergence.rs`, the spectrum by `tests/spectrum.rs`. Measure before changing it: `cargo run -p tunnels_audio --release --example envelope_suite -- <dir>` (see the crate docs in `tunnels_audio/src/lib.rs`).

## GUI architecture

- The console GUI sends `MetaCommand`s and `ControlMessage`s through a channel. The show loop processes them and emits `StateChange`s back to all listeners. **Unidirectional flow is a design rule, not a suggestion**: GUI → commands → show → state snapshots → GUI. GUI code does not write directly to show-facing atomics; instead it sends a `MetaCommand` that the show handles.
- `GuiState` (in `tunnels/src/gui_state.rs`) is the show → GUI surface. Fields the GUI should repaint on use `Notified<T>` / `NotifiedAtomic<T>` from `tunnels_lib::notified`, which wrap `ArcSwap<T>` / an `AtomicU64` (for any `T: AtomicBits`, a value that round-trips through a `u64`) and fire a `RepaintSignal` with the write. The one raw `ArcSwap<T>` is `animation_state`, read only by the animation visualizer, which animates every frame while visible; it is not a precedent for new fields.
- **Never force the GUI to repaint continuously to keep a value fresh.** State the GUI shows goes through a `Notified`/`NotifiedAtomic` `GuiState` field, stored only when the shown value changes (`NotifiedAtomic::store_if_changed`). A value produced on a real-time thread is relayed by the show tick, since the audio thread must never fire the repaint signal (egui's `request_repaint` takes a lock). Continuous repaint is only for a visible, genuinely animating view (an open envelope viewer, the animation visualizer) when there is no other option.
- The `RepaintSignal` is a `tunnels_lib::repaint::RepaintSignal` — `Arc<dyn Fn() + Send + Sync>`. The console wraps `egui::Context::request_repaint` inside the eframe creator closure; tests and headless callers use `noop_repaint()`.
- Shared GUI components live in `gui_common/`. Panel pattern: state struct + render struct with `GuiContext` for sending commands. Edge-triggered GUI state reported to the show (e.g. visualizer visibility) uses `gui_common::tracked::TrackedBool` to detect changes and avoid re-sending.

## The console/client connection is deliberately unversioned

The show frame stream carries no magic bytes and no version byte, and the
encoding is tagless: the schema is the Rust type, and it does not travel. A
client and the console it renders for are always the same build, because the
bootstrapper pushes the client binary from beside the console rather than
letting a machine keep one of its own. Skew is not a state the system can
reach without someone stepping around the deploy path.

So do not add version negotiation, a compatibility window, or anything else
that tolerates a client and a console disagreeing about the model. A change to
`ShowFrame`, or to anything reachable from it, needs both ends redeployed and
nothing more.

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `tunnels` | Main library: show loop, audio, clocks, MIDI, OSC, control dispatch |
| `tunnels_model` | The show model and its render: mixer, beams, tunnels, animations, clocks, palette |
| `tunnels_audio` | Audio input, resonator bank, role envelopes, spectrum, frame and ring buffers |
| `tunnels_lib` | Cross-crate primitives: number types, audio frames, color, smoothing, GUI repaint (`RepaintSignal`, `Notified`), transient indicator, bootstrap push protocol |
| `tunnels_net` | The network services a show is made of: the show frame stream |
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
| `golden_image` | Comparison of a rendered image against a checked-in golden |
