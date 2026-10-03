//! Settings startup watcher composition.

use super::*;

mod appearance;
mod apps_focus;
mod hardware;
mod network;
mod shell_input;
mod snapshots;
mod system;
mod updates_locale;

impl Settings {
    pub(super) fn start_watchers(
        cx: &mut Context<Self>,
        catalog_event_rx: async_channel::Receiver<()>,
    ) {
        Self::start_system_watchers(cx);
        Self::start_updates_locale_watchers(cx);
        Self::start_hardware_watchers(cx);
        Self::start_network_watchers(cx);
        Self::start_snapshot_loads(cx);
        Self::start_appearance_watchers(cx);
        Self::start_apps_focus_watchers(cx, catalog_event_rx);
        Self::start_shell_input_watchers(cx);
    }
}
