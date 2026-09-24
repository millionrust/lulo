//! Pure decision logic for the Mac Shortcuts page.
//!
//! Setup Assistant offers "Use Mac shortcuts in all apps" on its own page
//! right after Keyboard (owner decision; see `docs/decisions/0017-mac-keyboard.md`). The
//! toggle defaults on; the user can turn it off before Continue. Applying the
//! choice goes through the very function System Settings › Keyboard calls,
//! `rmac_keyboard::apply` (`docs/decisions/0017-mac-keyboard.md`), so
//! whichever privileged path is behind it — including its SR-13 revision,
//! which no longer needs a new login — applies here too. Everything in this
//! module is pure so the page's state logic is unit-tested without GPUI.

use rmac_keyboard::{MacKeyboard, Status};

/// Whether keyd is in a state where Mac shortcuts can be turned on at all.
/// Mirrors the same check System Settings uses to enable its switch.
pub fn available(status: &Status) -> bool {
    status.keyd_installed && status.helper_installed && status.foreign_keyd_configs.is_empty()
}

/// The keyboard state to apply for the page's toggle, or `None` when the
/// system already matches the toggle and nothing needs to change (so
/// Continue can move on without a privileged call at all).
pub fn target(status: &Status, enabled: bool) -> Option<MacKeyboard> {
    let mut target = status.state;
    target.shortcuts_in_all_apps = enabled && available(status);
    (target != status.state).then_some(target)
}

/// Why the toggle cannot be turned on, to explain instead of asking for a
/// password that would only fail. `None` means it can be offered normally.
pub fn unavailable_reason(status: &Status) -> Option<&'static str> {
    if available(status) {
        None
    } else if !status.keyd_installed {
        Some(
            "Mac shortcuts in all apps need the keyd package. \
             You can turn them on later in System Settings › Keyboard.",
        )
    } else if !status.foreign_keyd_configs.is_empty() {
        Some(
            "keyd already has another configuration, so Lulo OS leaves it alone. \
             You can turn Mac shortcuts on later in System Settings › Keyboard.",
        )
    } else {
        Some("Mac shortcuts in all apps are available when Lulo OS is installed from its package.")
    }
}

/// The note to show after a Continue attempt could not turn the toggle on
/// (the admin password was cancelled or denied, or the privileged helper
/// otherwise failed). Setup keeps going either way — this only explains why
/// the switch is now off.
pub fn declined_note(error: &dyn std::fmt::Display) -> String {
    format!(
        "Mac shortcuts in all apps weren't turned on ({error}). \
         You can turn them on later in System Settings › Keyboard."
    )
}
