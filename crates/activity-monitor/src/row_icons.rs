//! UIA-16: Activity Monitor rows need the Mac's leading app icon in front
//! of each process name. The kernel only gives a process a binary name
//! (`full_process_name`'s own result), so turning that into an icon means
//! walking the installed-application catalog — a disk scan of every
//! `.desktop` entry plus XDG icon-theme lookups for each one's `Icon=` —
//! far too slow to redo on the UI thread every refresh tick, and unneeded
//! to: process names barely change between ticks. The catalog is loaded
//! once, off thread, and kept process-wide; everything that doesn't match
//! an installed app (every background daemon and kernel thread, which is
//! most rows) shows the same generic app icon the Dock and App Drawer fall
//! back to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use gpui::Context;

/// Generic icon for a process that doesn't map to any installed app.
pub(crate) const GENERIC_ICON: &str = "icons/application.svg";

#[derive(Default)]
struct State {
    /// Executable basename (an installed app's `LaunchSpec::Command::
    /// program`, the same shape `full_process_name` resolves a running
    /// process to) mapped to that app's resolved icon path.
    by_program: HashMap<String, PathBuf>,
    ready: bool,
    loading: bool,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

/// The icon for a process named `process_name`, once the installed-app
/// catalog has loaded and has a match. `None` either while the catalog is
/// still loading (this kicks off that load, once, on the first call) or
/// once it has loaded and found nothing — callers show [`GENERIC_ICON`]
/// either way, so the two cases need no distinction here.
pub(crate) fn process_icon<T: 'static>(process_name: &str, cx: &Context<T>) -> Option<PathBuf> {
    let mut guard = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.ready {
        return guard.by_program.get(process_name).cloned();
    }
    if !guard.loading {
        guard.loading = true;
        drop(guard);
        spawn_load(cx);
    }
    None
}

fn spawn_load<T: 'static>(cx: &Context<T>) {
    cx.spawn(async move |this, cx| {
        let by_program = blocking::unblock(load_catalog).await;
        {
            let mut guard = state().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.by_program = by_program;
            guard.ready = true;
            guard.loading = false;
        }
        let _ = this.update(cx, |_, cx| cx.notify());
    })
    .detach();
}

fn load_catalog() -> HashMap<String, PathBuf> {
    let mut by_program = HashMap::new();
    for application in rmac_apps::discover().unwrap_or_default() {
        let Some(icon) = application.icon else {
            continue;
        };
        let rmac_apps::LaunchSpec::Command { program, .. } = &application.launch else {
            continue;
        };
        let Some(basename) = std::path::Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        by_program.entry(basename.to_owned()).or_insert(icon);
    }
    by_program
}

#[cfg(test)]
mod tests {
    use super::GENERIC_ICON;

    /// The fallback is a real bundled asset path, not a placeholder string
    /// that happens to compile — `rmac_ui::svg_icon` resolves it through
    /// the window's `AssetSource` exactly like the Dock's own fallback.
    #[test]
    fn generic_icon_is_an_asset_path() {
        assert!(GENERIC_ICON.starts_with("icons/"));
        assert!(GENERIC_ICON.ends_with(".svg"));
    }
}
