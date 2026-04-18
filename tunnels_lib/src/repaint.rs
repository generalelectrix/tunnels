//! Framework-agnostic handle for waking an immediate-mode GUI when shared
//! state changes. The concrete callback is installed by the GUI layer;
//! non-GUI crates depend only on the type alias.

use std::sync::Arc;

pub type RepaintSignal = Arc<dyn Fn() + Send + Sync>;

/// A repaint signal that does nothing.
pub fn noop_repaint() -> RepaintSignal {
    Arc::new(|| {})
}
