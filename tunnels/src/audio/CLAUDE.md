# Audio Subsystem

The audio subsystem is now extracted into the `tunnels_audio` crate. This module is a thin re-export layer that provides the adapter (`ShowEmitter`) bridging the audio crate's `EmitStateChange` trait to the show-level `EmitStateChange` trait.

## Architecture

See `tunnels_audio` crate for the full DSP architecture documentation.

## This module provides

- Re-exports of all `tunnels_audio` public types (so existing `crate::audio::*` paths work)
- `ShowEmitter<'a, T>` — newtype adapter wrapping show-level emitters for use with `AudioInput` methods

## Call site pattern

```rust
// Before (when audio was inline):
audio_input.update_state(delta_t, &mut emitter);

// After (with extracted crate):
audio_input.update_state(delta_t, &mut ShowEmitter(&mut emitter));
```
