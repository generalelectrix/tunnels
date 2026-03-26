# Tunnels

Tunnels is a live performance system for projecting immersive tunnels of light. It consists of a control server, render clients, and shared libraries — all running during a show in front of an audience.

## No panics during a show

All components of this system run live. A crash stops the show. Code running after initialization must not panic. If an operation can fail, log the error and recover gracefully — skip a frame, skip a shape, drop a message. Use `.unwrap()` and `.expect()` only during startup initialization where there is no meaningful recovery (e.g. GPU device creation, window creation, config parsing).

Mutex `.lock().unwrap()` is acceptable when there is no recovery path from a poisoned mutex.
