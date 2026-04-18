# Audio Module

Thin re-export layer over the `tunnels_audio` crate. Provides:

- `pub use tunnels_audio::*` so existing `crate::audio::*` paths work
- `ShowEmitter` — adapter bridging `tunnels_audio::EmitStateChange` to the show-level trait
