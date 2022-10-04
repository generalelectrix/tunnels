use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A helper state struct for managing the display of short-lived transients.
/// For example, clock tick events.
pub struct TransientIndicator {
    age: Option<Duration>,
    display_duration: Duration,
}

impl TransientIndicator {
    pub fn new(display_duration: Duration) -> Self {
        Self {
            age: None,
            display_duration,
        }
    }

    /// Update the state of this indicator.
    /// Return Some if we should emit a state change for the indicator.
    /// Return None if no action is necessary.
    pub fn update_state(&mut self, delta_t: Duration, transient_active: bool) -> Option<bool> {
        // if the transient is currently active, reset age and emit
        if transient_active {
            self.age = Some(Duration::ZERO);
            return Some(true);
        }
        if let Some(age) = self.age {
            let new_age = age + delta_t;
            if new_age > self.display_duration {
                self.age = None;
                return Some(false);
            }
            self.age = Some(new_age);
        }
        None
    }

    /// Return true if this indicator should currently display a transient or not.
    pub fn state(&self) -> bool {
        self.age.is_some()
    }

    /// Cancel any outstanding transient.
    pub fn reset(&mut self) {
        self.age = None;
    }
}
