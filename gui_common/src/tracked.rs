//! A boolean whose current value is stored and whose changes are observed.
//!
//! The primary role is state storage; change detection is emitted as a
//! by-product of each `update`. The pattern generalizes to `Tracked<T>` /
//! `Changed<T>` if we ever need it for non-bool values.

/// A boolean that remembers its current value across frames.
pub struct TrackedBool {
    current: bool,
}

impl TrackedBool {
    pub fn new(initial: bool) -> Self {
        Self { current: initial }
    }

    pub fn get(&self) -> bool {
        self.current
    }

    /// Replace the stored value and report whether it changed.
    pub fn update(&mut self, value: bool) -> ChangedBool {
        let changed = value != self.current;
        self.current = value;
        ChangedBool { changed, value }
    }
}

/// The outcome of a `TrackedBool::update`.
#[derive(Copy, Clone, Debug)]
#[must_use]
pub struct ChangedBool {
    pub changed: bool,
    pub value: bool,
}

impl ChangedBool {
    /// Run `f` with the new value iff this update represented a change.
    pub fn if_changed(self, f: impl FnOnce(bool)) {
        if self.changed {
            f(self.value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_matching_initial_is_not_changed() {
        let mut t = TrackedBool::new(false);
        let c = t.update(false);
        assert!(!c.changed);
        assert!(!c.value);
        assert!(!t.get());
    }

    #[test]
    fn update_differing_from_initial_is_changed() {
        let mut t = TrackedBool::new(false);
        let c = t.update(true);
        assert!(c.changed);
        assert!(c.value);
        assert!(t.get());
    }

    #[test]
    fn steady_state_after_change_is_not_changed() {
        let mut t = TrackedBool::new(false);
        assert!(t.update(true).changed);
        assert!(!t.update(true).changed);
        assert!(!t.update(true).changed);
    }

    #[test]
    fn toggling_reports_each_edge() {
        let mut t = TrackedBool::new(false);
        assert!(t.update(true).changed);
        assert!(t.update(false).changed);
        assert!(t.update(true).changed);
    }

    #[test]
    fn if_changed_fires_only_on_change() {
        let mut fired_with: Option<bool> = None;
        let mut t = TrackedBool::new(false);

        t.update(false).if_changed(|v| fired_with = Some(v));
        assert_eq!(fired_with, None);

        t.update(true).if_changed(|v| fired_with = Some(v));
        assert_eq!(fired_with, Some(true));

        fired_with = None;
        t.update(true).if_changed(|v| fired_with = Some(v));
        assert_eq!(fired_with, None);
    }
}
