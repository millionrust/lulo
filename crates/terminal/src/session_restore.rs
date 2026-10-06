//! Application ▸ Quit and Keep Windows (TERM-22, ⌥⌘Q): what one window's
//! tabs need to reopen somewhere else — each tab's working directory, what
//! it execs (if anything, instead of a plain shell), its colour profile,
//! and a bounded snapshot of its visible buffer text so the reopened tab
//! still looks like where the user left it.
//!
//! No live process state (the PTY, job state, the foreground command)
//! survives a restore — only what a plain new session plus some printed
//! text can reproduce. The whole window is carried as one JSON string in a
//! single `--restore=` argument (see `cli::restore_flag`/`parse_restore_flag`),
//! the same way every other Terminal window is opened
//! (`rmac_ui::boot_app_instance`'s per-window argument lists), so relaunch
//! needs no extra IPC of its own.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Replayed scrollback is printed as plain inert text, never reconnected to
/// a process, so it is capped well below the live scrollback limit —
/// enough to recognise the session, not a cost to inject or to carry in one
/// argv string.
pub(crate) const MAX_REPLAYED_SCROLLBACK_BYTES: usize = 16_384;

/// How many tabs one window's `--restore=` saves/restores. Generous for a
/// terminal window, and well short of `MAX_TABS`.
pub(crate) const MAX_RESTORED_TABS: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct RestoreTab {
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) program: Option<String>,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) profile: usize,
    #[serde(default)]
    pub(crate) scrollback: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct RestoreWindow {
    pub(crate) tabs: Vec<RestoreTab>,
}

impl RestoreWindow {
    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

/// Bound replayed scrollback to [`MAX_REPLAYED_SCROLLBACK_BYTES`], keeping
/// the most recent text and cutting only at a `char` boundary.
pub(crate) fn bound_scrollback(text: &str) -> &str {
    if text.len() <= MAX_REPLAYED_SCROLLBACK_BYTES {
        return text;
    }
    let mut start = text.len() - MAX_REPLAYED_SCROLLBACK_BYTES;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// `RestoreWindow` as one compact JSON string (no embedded raw newlines —
/// `serde_json` escapes control characters inside its string values), fit
/// to travel as a single argv element.
pub(crate) fn encode(window: &RestoreWindow) -> Option<String> {
    serde_json::to_string(window).ok()
}

pub(crate) fn decode(text: &str) -> Option<RestoreWindow> {
    serde_json::from_str(text).ok()
}

fn xdg_state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
}

/// Where Quit and Keep Windows saves the kept session, read back exactly
/// once on the next launch.
pub(crate) fn kept_windows_path() -> Option<PathBuf> {
    xdg_state_dir().map(|dir| dir.join("rmac-terminal/kept-windows.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restored_window_round_trips_through_json() {
        let window = RestoreWindow {
            tabs: vec![
                RestoreTab {
                    cwd: Some(PathBuf::from("/home/user/project")),
                    program: None,
                    args: Vec::new(),
                    profile: 2,
                    scrollback: "$ ls\nCargo.toml\n".to_string(),
                },
                RestoreTab {
                    cwd: Some(PathBuf::from("/tmp")),
                    program: Some("vim".to_string()),
                    args: vec!["notes.txt".to_string()],
                    profile: 0,
                    scrollback: String::new(),
                },
            ],
        };
        let encoded = encode(&window).expect("encodes");
        assert!(!encoded.contains('\n'), "fit for one argv element");
        assert_eq!(decode(&encoded), Some(window));
    }

    #[test]
    fn malformed_or_empty_restore_text_decodes_to_nothing() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("not json"), None);
        assert_eq!(decode("{}"), None);
    }

    #[test]
    fn empty_windows_are_recognised_so_callers_can_skip_them() {
        assert!(RestoreWindow::default().is_empty());
        assert!(!RestoreWindow {
            tabs: vec![RestoreTab::default()],
        }
        .is_empty());
    }

    #[test]
    fn scrollback_is_bounded_to_the_most_recent_text_on_a_char_boundary() {
        let short = "hello";
        assert_eq!(bound_scrollback(short), short);

        let long: String = "é".repeat(MAX_REPLAYED_SCROLLBACK_BYTES); // 2 bytes each
        let bounded = bound_scrollback(&long);
        assert!(bounded.len() <= MAX_REPLAYED_SCROLLBACK_BYTES);
        assert!(long.ends_with(bounded));
    }
}
