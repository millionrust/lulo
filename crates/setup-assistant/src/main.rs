//! rmac Setup Assistant.
//!
//! `rmac-setup-assistant --first-login` (the session unit) exits at once when
//! setup has already run; without the flag (System Settings › General) it
//! always opens.

mod view;

use std::borrow::Cow;

use gpui::{actions, AssetSource, KeyBinding, Result, SharedString};
use rmac_setup_assistant::flow::Availability;
use rmac_setup_assistant::{marker, services, APP_ID};

use crate::view::{SetupView, WINDOW_SIZE};

// Return activates the current screen's default (rightmost) button, the
// same as macOS's own Setup Assistant. Without this a keyboard-only user
// had no way to advance past Welcome at all: its round "Get Started" button
// is a plain div with no track_focus (a separate, larger gap -- see
// docs/keyboard-audit.md), so Tab never reaches it and only Skip Setup
// (which ends the whole assistant) was keyboard-reachable there.
actions!(setup_assistant, [Continue]);

/// The page icons (lucide, as in design-lab/setup-assistant.html), served
/// under `setup/`.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/icons"]
#[include = "*.svg"]
struct Icons;

struct SetupAssets;

impl AssetSource for SetupAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(path
            .strip_prefix("setup/")
            .and_then(Icons::get)
            .map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Icons::iter()
            .map(|name| SharedString::from(format!("setup/{name}")))
            .filter(|name| name.starts_with(path))
            .collect())
    }
}

fn main() {
    let first_login = std::env::args()
        .skip(1)
        .any(|argument| argument == "--first-login");
    if first_login && marker::is_complete() {
        return;
    }
    let wifi = rmac_network::snapshot().ok();
    let account = services::account().ok();
    let availability = Availability {
        wifi_needed: wifi
            .as_ref()
            .is_some_and(|wifi| wifi.available && wifi.current_ssid.is_none()),
        accounts: account.is_some(),
    };
    let (width, height) = WINDOW_SIZE;
    rmac_ui::boot_app_with_assets(
        APP_ID,
        rmac_ui::layered_assets(SetupAssets),
        "Setup Assistant",
        width,
        height,
        move |window, cx| {
            let view = SetupView::new(availability, wifi, account, window, cx);
            cx.bind_keys([KeyBinding::new(
                rmac_ui::shortcuts::ENTER.keystroke,
                Continue,
                Some("SetupAssistant"),
            )]);
            view
        },
    );
}
