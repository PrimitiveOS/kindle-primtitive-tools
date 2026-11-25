//! Restart the launcher by holding both page-turn buttons for two seconds. The launcher reports each button's press and
//! release; a hold is decided by `tick` once both buttons have stayed down for the whole time. Times are milliseconds.

/// How long both buttons must stay down.
pub const HOLD_MS: u64 = 2_000;
/// The evdev codes of the two page-turn keys (KEY_PAGEUP and KEY_PAGEDOWN).
pub const PAGE_UP: u16 = 104;
pub const PAGE_DOWN: u16 = 109;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PageChord {
    up_down: bool,
    down_down: bool,
    /// When both buttons went down (cleared as soon as one comes up).
    both_since: Option<u64>,
    /// The hold was already reported for this press; a new hold needs both buttons up and down again.
    fired: bool,
}

impl PageChord {
    /// A press (`down`) or release of a key at `now`. Other keys are ignored.
    pub fn key(&mut self, code: u16, down: bool, now: u64) {
        match code {
            PAGE_UP => self.up_down = down,
            PAGE_DOWN => self.down_down = down,
            _ => return,
        }
        if self.up_down && self.down_down {
            if self.both_since.is_none() {
                self.both_since = Some(now);
            }
        } else {
            self.both_since = None;
            // A new hold needs both buttons up first, so letting go of one does not allow a second restart.
            if !self.up_down && !self.down_down {
                self.fired = false;
            }
        }
    }

    /// The time at which the hold will be complete, while it is still pending.
    pub fn deadline(&self) -> Option<u64> {
        if self.fired {
            None
        } else {
            self.both_since.map(|since| since + HOLD_MS)
        }
    }

    /// True once, when both buttons have been down for `HOLD_MS`.
    pub fn tick(&mut self, now: u64) -> bool {
        match self.both_since {
            Some(since) if !self.fired && now >= since + HOLD_MS => {
                self.fired = true;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_both_buttons_for_two_seconds_fires_once() {
        let mut chord = PageChord::default();
        chord.key(PAGE_UP, true, 1_000);
        chord.key(PAGE_DOWN, true, 1_200);
        assert_eq!(chord.deadline(), Some(3_200), "the hold starts when the second button goes down");
        assert!(!chord.tick(3_199));
        assert!(chord.tick(3_200));
        assert!(!chord.tick(9_000), "one hold fires once");
        assert_eq!(chord.deadline(), None);
    }

    #[test]
    fn letting_go_early_does_not_fire() {
        let mut chord = PageChord::default();
        chord.key(PAGE_UP, true, 0);
        chord.key(PAGE_DOWN, true, 100);
        chord.key(PAGE_DOWN, false, 1_500);
        assert!(!chord.tick(5_000));
        assert_eq!(chord.deadline(), None);
    }

    #[test]
    fn one_button_alone_does_not_fire() {
        let mut chord = PageChord::default();
        chord.key(PAGE_UP, true, 0);
        assert_eq!(chord.deadline(), None);
        assert!(!chord.tick(10_000));
    }

    #[test]
    fn a_new_hold_needs_both_buttons_up_first() {
        let mut chord = PageChord::default();
        chord.key(PAGE_UP, true, 0);
        chord.key(PAGE_DOWN, true, 0);
        assert!(chord.tick(2_000));
        // Still holding: pressing the other button again does not start another hold.
        chord.key(PAGE_UP, false, 3_000);
        chord.key(PAGE_UP, true, 3_100);
        assert!(!chord.tick(9_000));
        chord.key(PAGE_UP, false, 9_500);
        chord.key(PAGE_DOWN, false, 9_600);
        chord.key(PAGE_UP, true, 10_000);
        chord.key(PAGE_DOWN, true, 10_000);
        assert!(chord.tick(12_000));
    }

    #[test]
    fn other_keys_are_ignored() {
        let mut chord = PageChord::default();
        chord.key(116, true, 0);
        chord.key(PAGE_UP, true, 0);
        chord.key(PAGE_DOWN, true, 0);
        assert!(chord.tick(2_000));
    }
}
