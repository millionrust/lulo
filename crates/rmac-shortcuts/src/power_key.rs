//! What a press of the power button does in a Lulo OS session.
//!
//! logind's default on Ubuntu is `HandlePowerKey=poweroff`: one short press
//! ends the session at once, and unsaved work goes with it. On a Mac a
//! short press sleeps (and so locks) the computer, and holding the button
//! shows the Restart / Sleep / Shut Down dialog.
//!
//! GNOME keeps the Mac's behaviour by holding logind's `handle-power-key`
//! *block* inhibitor for as long as the session runs and handling the key
//! itself. Lulo OS does the same: the lock coordinator holds the inhibitor
//! (see `lock::coordinate`), niri passes each press to it through the
//! `power-key` dispatch socket, and this module decides what the press
//! means. Nothing changes logind's configuration, so the Ubuntu session is
//! untouched, and the button falls back to logind's own action whenever the
//! coordinator is not running.
//!
//! The ACPI power button of an x86 laptop reports press and release
//! together, the moment the button goes down, so a long press cannot be told
//! from a short one (holding it for four seconds is the firmware's forced
//! power-off, which no software sees). A second press within
//! [`DOUBLE_PRESS_WINDOW`] therefore stands in for the Mac's long press and
//! shows the shutdown dialog; a single press locks, then sleeps, once that
//! window has passed.

use std::time::{Duration, Instant};

/// How long a first press waits for a second one before the computer
/// sleeps. S: not measured on the Mac, whose long press has no equivalent.
pub const DOUBLE_PRESS_WINDOW: Duration = Duration::from_millis(800);

/// Presses this soon after waking are ignored. Many laptops deliver the
/// press that woke them as a key event once they resume, which would
/// otherwise send the computer straight back to sleep.
pub const WAKE_GUARD: Duration = Duration::from_secs(2);

/// The dispatch socket niri sends each power-button press to.
pub const POWER_KEY_SHORTCUT: &str = "power-key";
/// The dispatch socket the menu bar listens on for the shutdown dialog.
pub const SHUTDOWN_DIALOG_SHORTCUT: &str = "shutdown-dialog";

/// The logind inhibitor that leaves the button to the session. It is a
/// *block* inhibitor on a key, not on shutdown itself: `systemctl poweroff`,
/// the menu bar's Shut Down and UPower's critical-battery action still work.
pub const INHIBIT_WHAT: &str = "handle-power-key";
pub const INHIBIT_WHY: &str =
    "The power button sleeps; press it twice for Restart, Sleep or Shut Down";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerKeyAction {
    /// Do nothing now.
    Ignore,
    /// Wait until the given moment for a second press, then ask again
    /// through [`PowerKey::deadline_passed`].
    WaitUntil(Instant),
    /// Show the Restart / Sleep / Cancel / Shut Down dialog.
    ShowShutdownDialog,
    /// Sleep, locking the session first unless it is already locked.
    Sleep { lock_first: bool },
}

/// The power button's state between presses. Pure, so it is unit tested on
/// every host.
#[derive(Debug, Default)]
pub struct PowerKey {
    pending_since: Option<Instant>,
    woke_at: Option<Instant>,
}

impl PowerKey {
    /// The button was pressed at `now`.
    pub fn press(&mut self, now: Instant, locked: bool) -> PowerKeyAction {
        if self
            .woke_at
            .is_some_and(|woke| now.saturating_duration_since(woke) < WAKE_GUARD)
        {
            return PowerKeyAction::Ignore;
        }
        if locked {
            // The dialog cannot appear over the lock screen, and the Mac
            // just sleeps its display here.
            self.pending_since = None;
            return PowerKeyAction::Sleep { lock_first: false };
        }
        match self.pending_since.take() {
            Some(first) if now.saturating_duration_since(first) < DOUBLE_PRESS_WINDOW => {
                PowerKeyAction::ShowShutdownDialog
            }
            _ => {
                self.pending_since = Some(now);
                PowerKeyAction::WaitUntil(now + DOUBLE_PRESS_WINDOW)
            }
        }
    }

    /// The moment a [`PowerKeyAction::WaitUntil`] named has come.
    pub fn deadline_passed(&mut self, now: Instant, locked: bool) -> PowerKeyAction {
        match self.pending_since {
            Some(first) if now.saturating_duration_since(first) >= DOUBLE_PRESS_WINDOW => {
                self.pending_since = None;
                PowerKeyAction::Sleep {
                    lock_first: !locked,
                }
            }
            Some(first) => PowerKeyAction::WaitUntil(first + DOUBLE_PRESS_WINDOW),
            None => PowerKeyAction::Ignore,
        }
    }

    /// When a single press will turn into sleep, if one is waiting.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending_since.map(|first| first + DOUBLE_PRESS_WINDOW)
    }

    /// The computer is about to sleep for some other reason (a lid close,
    /// the menu bar's Sleep): a waiting press is moot.
    pub fn sleeping(&mut self) {
        self.pending_since = None;
    }

    /// The computer woke at `now`.
    pub fn woke(&mut self, now: Instant) {
        self.pending_since = None;
        self.woke_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_single_press_locks_then_sleeps_once_the_window_passes() {
        let start = Instant::now();
        let mut key = PowerKey::default();
        assert_eq!(
            key.press(start, false),
            PowerKeyAction::WaitUntil(start + DOUBLE_PRESS_WINDOW)
        );
        assert_eq!(key.deadline(), Some(start + DOUBLE_PRESS_WINDOW));
        // A timer that fires early just waits again.
        assert_eq!(
            key.deadline_passed(start + 10 * MS, false),
            PowerKeyAction::WaitUntil(start + DOUBLE_PRESS_WINDOW)
        );
        assert_eq!(
            key.deadline_passed(start + DOUBLE_PRESS_WINDOW, false),
            PowerKeyAction::Sleep { lock_first: true }
        );
        assert_eq!(key.deadline(), None);
        assert_eq!(
            key.deadline_passed(start + 2 * DOUBLE_PRESS_WINDOW, false),
            PowerKeyAction::Ignore
        );
    }

    #[test]
    fn a_second_press_inside_the_window_shows_the_dialog_instead_of_sleeping() {
        let start = Instant::now();
        let mut key = PowerKey::default();
        key.press(start, false);
        assert_eq!(
            key.press(start + 300 * MS, false),
            PowerKeyAction::ShowShutdownDialog
        );
        // Nothing is left waiting, so the computer does not sleep after all.
        assert_eq!(key.deadline(), None);
        assert_eq!(
            key.deadline_passed(start + DOUBLE_PRESS_WINDOW, false),
            PowerKeyAction::Ignore
        );
        // A third press starts over.
        assert_eq!(
            key.press(start + 400 * MS, false),
            PowerKeyAction::WaitUntil(start + 400 * MS + DOUBLE_PRESS_WINDOW)
        );
    }

    #[test]
    fn a_late_second_press_is_a_new_first_press() {
        let start = Instant::now();
        let mut key = PowerKey::default();
        key.press(start, false);
        let late = start + DOUBLE_PRESS_WINDOW + MS;
        assert_eq!(
            key.press(late, false),
            PowerKeyAction::WaitUntil(late + DOUBLE_PRESS_WINDOW)
        );
    }

    #[test]
    fn a_press_on_the_lock_screen_sleeps_at_once() {
        let start = Instant::now();
        let mut key = PowerKey::default();
        assert_eq!(
            key.press(start, true),
            PowerKeyAction::Sleep { lock_first: false }
        );
        // Locked while a press waited (the idle lock, ⌃⌘Q): no second lock.
        key.press(start + DOUBLE_PRESS_WINDOW, false);
        assert_eq!(
            key.deadline_passed(start + 2 * DOUBLE_PRESS_WINDOW, true),
            PowerKeyAction::Sleep { lock_first: false }
        );
    }

    #[test]
    fn the_press_that_woke_the_computer_does_not_put_it_back_to_sleep() {
        let start = Instant::now();
        let mut key = PowerKey::default();
        key.press(start, false);
        key.sleeping();
        assert_eq!(key.deadline(), None);
        let woke = start + Duration::from_secs(60);
        key.woke(woke);
        assert_eq!(key.press(woke + 100 * MS, false), PowerKeyAction::Ignore);
        assert_eq!(key.press(woke + 100 * MS, true), PowerKeyAction::Ignore);
        assert_eq!(
            key.press(woke + WAKE_GUARD, false),
            PowerKeyAction::WaitUntil(woke + WAKE_GUARD + DOUBLE_PRESS_WINDOW)
        );
    }
}
