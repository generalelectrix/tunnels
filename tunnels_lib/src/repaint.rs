//! GUI repaint signal — a framework-agnostic handle writers can call to wake
//! an immediate-mode GUI when shared state changes.
//!
//! The concrete callback is installed by the GUI (typically `ctx.request_repaint()`
//! on an `egui::Context`). Non-GUI crates depend only on the type alias.

use std::sync::Arc;

pub type RepaintSignal = Arc<dyn Fn() + Send + Sync>;

/// A repaint signal that does nothing. Used by tests and headless code paths.
pub fn noop_repaint() -> RepaintSignal {
    Arc::new(|| {})
}
