//! The menu bar's side of keeping unsaved work through a shutdown or sleep it
//! did not start: which apps hold unsaved work, when to hold logind's delay
//! inhibitor, and what to do when logind announces it is about to act. Kept
//! free of GPUI and D-Bus so it is unit tested on every host.
//!
//! macOS asks every app to save before a restart it starts itself, which the
//! menu bar's Log Out, Restart and Shut Down already do. Anything else ends
//! the session without asking: macOS relies on each app having autosaved,
//! and so do we. A delay inhibitor never refuses a shutdown or a sleep; it
//! only holds it for up to logind's `InhibitDelayMaxSec` (5 s by default)
//! while every app with unsaved work writes its recovery drafts. The menu
//! bar takes no "block" inhibitor: no Mac app can stop a forced shutdown,
//! a low-battery power-off or a lid-close sleep, and one that could would
//! lose more work than it saved once the battery ran out.

use std::collections::BTreeSet;
use std::time::Duration;

/// What the inhibitor delays: both ending the session and sleeping, since a
/// battery can run out while asleep.
pub const INHIBIT_WHAT: &str = "shutdown:sleep";
pub const INHIBIT_WHO: &str = "Lulo OS";
pub const INHIBIT_WHY: &str = "Saving recovery drafts of unsaved documents";
pub const INHIBIT_MODE: &str = "delay";

/// logind's default `InhibitDelayMaxSec`, used when the setting cannot be
/// read.
const DEFAULT_DELAY_MAX: Duration = Duration::from_secs(5);
/// Released this long before logind would stop waiting, so the release is
/// what lets it continue.
const RELEASE_MARGIN: Duration = Duration::from_millis(500);
const MIN_BUDGET: Duration = Duration::from_millis(250);

/// How long the menu bar spends keeping unsaved work once logind announces
/// a shutdown or sleep, given logind's `InhibitDelayMaxUSec`.
pub fn preserve_budget(delay_max: Option<Duration>) -> Duration {
    let delay_max = delay_max
        .filter(|delay| !delay.is_zero())
        .unwrap_or(DEFAULT_DELAY_MAX);
    delay_max
        .saturating_sub(RELEASE_MARGIN)
        .max(MIN_BUDGET.min(delay_max))
}

/// The unsaved-work bus names owned right now.
#[derive(Debug, Default)]
pub struct UnsavedApps {
    names: BTreeSet<String>,
}

impl UnsavedApps {
    pub fn seed(&mut self, names: impl IntoIterator<Item = String>) {
        self.names.extend(names);
    }

    pub fn update(&mut self, name: String, owned: bool) {
        if owned {
            self.names.insert(name);
        } else {
            self.names.remove(&name);
        }
    }

    pub fn any(&self) -> bool {
        !self.names.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.names.iter().cloned().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prepare {
    Shutdown,
    Sleep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardAction {
    /// Ask every app with unsaved work to write its drafts now; before a
    /// shutdown also ask every window to close, as Log Out does. Then
    /// release the inhibitor so logind continues.
    PreserveThenRelease { close_windows: bool },
    /// Nothing is held, so logind is not waiting for us.
    Nothing,
}

/// What a `PrepareForShutdown` or `PrepareForSleep` with `start == true`
/// asks of the menu bar.
pub fn on_prepare(kind: Prepare, holding: bool) -> GuardAction {
    if holding {
        GuardAction::PreserveThenRelease {
            close_windows: kind == Prepare::Shutdown,
        }
    } else {
        GuardAction::Nothing
    }
}

/// Whether the inhibitor should be held: while some app has unsaved work,
/// except between logind's announcement and its completion (a lock taken
/// then would only hold up the shutdown or sleep already under way).
pub fn wants_inhibitor(any_unsaved: bool, preparing: bool) -> bool {
    any_unsaved && !preparing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_leaves_logind_a_margin_of_its_delay() {
        assert_eq!(preserve_budget(None), Duration::from_millis(4500));
        assert_eq!(
            preserve_budget(Some(Duration::from_secs(5))),
            Duration::from_millis(4500)
        );
        assert_eq!(
            preserve_budget(Some(Duration::from_secs(30))),
            Duration::from_millis(29_500)
        );
        // A tiny configured delay still gets what it allows.
        assert_eq!(
            preserve_budget(Some(Duration::from_millis(600))),
            Duration::from_millis(250)
        );
        assert_eq!(
            preserve_budget(Some(Duration::from_millis(100))),
            Duration::from_millis(100)
        );
        assert_eq!(
            preserve_budget(Some(Duration::ZERO)),
            Duration::from_millis(4500)
        );
    }

    #[test]
    fn the_inhibitor_follows_unsaved_apps_until_logind_acts() {
        let mut apps = UnsavedApps::default();
        assert!(!wants_inhibitor(apps.any(), false));
        apps.seed(["org.rmac.UnsavedWork.p10".to_string()]);
        assert!(wants_inhibitor(apps.any(), false));
        apps.update("org.rmac.UnsavedWork.p20".into(), true);
        apps.update("org.rmac.UnsavedWork.p10".into(), false);
        assert_eq!(apps.names(), vec!["org.rmac.UnsavedWork.p20".to_string()]);
        assert!(wants_inhibitor(apps.any(), false));
        // Not retaken while a shutdown or sleep is under way.
        assert!(!wants_inhibitor(apps.any(), true));
        apps.update("org.rmac.UnsavedWork.p20".into(), false);
        assert!(!wants_inhibitor(apps.any(), false));
    }

    #[test]
    fn only_a_held_inhibitor_makes_logind_wait_and_only_shutdown_closes_windows() {
        assert_eq!(
            on_prepare(Prepare::Shutdown, true),
            GuardAction::PreserveThenRelease {
                close_windows: true
            }
        );
        assert_eq!(
            on_prepare(Prepare::Sleep, true),
            GuardAction::PreserveThenRelease {
                close_windows: false
            }
        );
        assert_eq!(on_prepare(Prepare::Shutdown, false), GuardAction::Nothing);
        assert_eq!(on_prepare(Prepare::Sleep, false), GuardAction::Nothing);
    }
}
