//! The power key as the launcher sees it. The launcher grabs the key device, so every press reaches this state
//! machine before the Kindle's power daemon does. A press is a double press when a second press follows within
//! `DOUBLE_WINDOW_MS`; otherwise it is a single press, and the launcher decides that once the window has passed.
//! Times are milliseconds on a monotonic clock, passed in, so the rules are tested without a device.

/// How long after a press a second press still counts as a double press.
pub const DOUBLE_WINDOW_MS: u64 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Sleep the screen.
    Single,
    /// Quit the open app and restart the launcher (after the owner confirms).
    Double,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PowerKey {
    /// The time of a press that has not yet been decided as single or double.
    pending: Option<u64>,
}

impl PowerKey {
    /// A press at `now`. A second press inside the window is a double press, decided at once. A press that finds an
    /// earlier press still pending past its window decides that earlier press as single, and then waits itself.
    /// Otherwise the press waits for `tick`.
    pub fn press(&mut self, now: u64) -> Option<Gesture> {
        match self.pending.take() {
            Some(first) if now.saturating_sub(first) <= DOUBLE_WINDOW_MS => Some(Gesture::Double),
            Some(_) => {
                self.pending = Some(now);
                Some(Gesture::Single)
            }
            None => {
                self.pending = Some(now);
                None
            }
        }
    }

    /// The time at which a pending press will be decided as a single press, if one is pending.
    pub fn deadline(&self) -> Option<u64> {
        self.pending.map(|first| first + DOUBLE_WINDOW_MS + 1)
    }

    /// Decide a pending press as a single press once the window has passed.
    pub fn tick(&mut self, now: u64) -> Option<Gesture> {
        match self.pending {
            Some(first) if now.saturating_sub(first) > DOUBLE_WINDOW_MS => {
                self.pending = None;
                Some(Gesture::Single)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_press_is_decided_only_after_the_window() {
        let mut key = PowerKey::default();
        assert_eq!(key.press(1_000), None, "the launcher waits to see whether a second press follows");
        assert_eq!(key.tick(1_000 + DOUBLE_WINDOW_MS), None, "still inside the window");
        assert_eq!(key.tick(1_000 + DOUBLE_WINDOW_MS + 1), Some(Gesture::Single));
        assert_eq!(key.tick(5_000), None, "a decided press is not decided twice");
    }

    #[test]
    fn two_presses_inside_the_window_are_a_double_press() {
        let mut key = PowerKey::default();
        assert_eq!(key.press(1_000), None);
        assert_eq!(key.press(1_000 + DOUBLE_WINDOW_MS), Some(Gesture::Double), "the window's edge counts");
        assert_eq!(key.tick(5_000), None, "a double press leaves nothing pending");
    }

    #[test]
    fn a_press_after_the_window_starts_a_new_gesture() {
        let mut key = PowerKey::default();
        key.press(1_000);
        assert_eq!(key.tick(2_000), Some(Gesture::Single));
        assert_eq!(key.press(3_000), None, "a new press waits again");
        assert_eq!(key.press(3_100), Some(Gesture::Double));
    }

    #[test]
    fn a_late_second_press_decides_the_first_as_single_and_starts_its_own() {
        let mut key = PowerKey::default();
        key.press(1_000);
        // The first press was not decided before the second arrived late: it is single, and the second is a new press.
        assert_eq!(key.press(1_000 + DOUBLE_WINDOW_MS + 50), Some(Gesture::Single));
        assert_eq!(key.deadline(), Some(1_000 + DOUBLE_WINDOW_MS + 50 + DOUBLE_WINDOW_MS + 1));
    }

    #[test]
    fn the_deadline_is_when_a_pending_press_becomes_single() {
        let mut key = PowerKey::default();
        assert_eq!(key.deadline(), None);
        key.press(10_000);
        let at = key.deadline().unwrap();
        assert_eq!(key.tick(at - 1), None);
        assert_eq!(key.tick(at), Some(Gesture::Single));
    }
}
