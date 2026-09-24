//! Force Quit Applications (⌥⌘⎋ and the logo menu's Force Quit…): the list
//! of running applications, its order and selection, and the one
//! "not responding" signal Linux gives without the app's cooperation.
//!
//! Nothing here touches GPUI, sockets, or niri, so it is tested everywhere.
//! Geometry comes from design-lab/force-quit.html (macOS 26.2, measured
//! 2026-09-24).

use rmac_compositor::Snapshot;

use crate::model::is_switchable;

/// The Force Quit window's own app id. It is a system dialogue, so the
/// switcher and the Dock leave it out of their application lists.
pub const APP_ID: &str = "org.rmac.ForceQuit";
pub const TITLE: &str = "Force Quit Applications";
pub const INSTRUCTION: &str =
    "If an app doesn\u{2019}t respond for a while, select its name and click Force Quit.";
pub const FOOTER: &str = "You can open this window by pressing Command-Option-Escape.";
pub const NOT_RESPONDING: &str = "(Not Responding)";

/// Window and content geometry in logical points.
pub mod layout {
    pub const WIDTH: f32 = 370.0;
    pub const HEIGHT: f32 = 310.0;
    pub const TITLE_LEFT: f32 = 82.0;
    pub const TITLE_TOP: f32 = 8.0;
    pub const TITLE_HEIGHT: f32 = 16.0;
    pub const TEXT_LEFT: f32 = 19.0;
    pub const INSTRUCTION_TOP: f32 = 51.0;
    pub const INSTRUCTION_WIDTH: f32 = 332.0;
    pub const INSTRUCTION_LINE: f32 = 17.0;
    pub const LIST_LEFT: f32 = 20.0;
    pub const LIST_TOP: f32 = 92.0;
    pub const LIST_WIDTH: f32 = 330.0;
    pub const LIST_HEIGHT: f32 = 158.0;
    pub const ROW_HEIGHT: f32 = 22.0;
    /// Icon inset from the row's left edge and top.
    pub const ICON_LEFT: f32 = 7.0;
    pub const ICON_TOP: f32 = 2.0;
    pub const ICON: f32 = 18.0;
    /// The name starts 2 after the icon.
    pub const NAME_LEFT: f32 = 27.0;
    pub const FOOTER_TOP: f32 = 265.0;
    pub const FOOTER_WIDTH: f32 = 236.0;
    pub const FOOTER_LINE: f32 = 14.0;
    pub const BUTTON_LEFT: f32 = 261.0;
    pub const BUTTON_TOP: f32 = 265.0;
    pub const BUTTON_WIDTH: f32 = 93.0;
    pub const BUTTON_HEIGHT: f32 = 30.0;
}

/// One running application in the list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub app_id: String,
    pub name: String,
    /// Every distinct process that owns one of its windows.
    pub pids: Vec<u32>,
    pub not_responding: bool,
}

impl Entry {
    /// Nothing to signal: niri reported no process for any window.
    pub fn can_force_quit(&self) -> bool {
        !self.pids.is_empty()
    }
}

/// Running applications as the Mac lists them: alphabetically, with the file
/// manager (Finder there, Files here) last. `name` resolves the display
/// name; `stopped` answers whether a process has stopped responding.
pub fn entries(
    snapshot: &Snapshot,
    name: impl Fn(&str) -> String,
    stopped: impl Fn(u32) -> bool,
) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for window in &snapshot.windows {
        let Some(app_id) = window
            .app_id
            .as_deref()
            .filter(|id| is_switchable(id) && *id != APP_ID)
        else {
            continue;
        };
        let pid = window
            .pid
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 1);
        let index = match entries.iter().position(|entry| entry.app_id == app_id) {
            Some(index) => index,
            None => {
                entries.push(Entry {
                    app_id: app_id.to_owned(),
                    name: name(app_id),
                    pids: Vec::new(),
                    not_responding: false,
                });
                entries.len() - 1
            }
        };
        if let Some(pid) = pid {
            let entry = &mut entries[index];
            if !entry.pids.contains(&pid) {
                entry.pids.push(pid);
            }
        }
    }
    for entry in &mut entries {
        entry.pids.sort_unstable();
        entry.not_responding = entry.pids.iter().any(|pid| stopped(*pid));
    }
    entries.sort_by(|left, right| {
        let files = |entry: &Entry| entry.app_id == rmac_apps::identity::FILES;
        files(left)
            .cmp(&files(right))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.app_id.cmp(&right.app_id))
    });
    entries
}

/// The app that was frontmost before Force Quit opened, which starts
/// selected as on the Mac.
pub fn frontmost_app(snapshot: &Snapshot) -> Option<String> {
    snapshot
        .focus
        .window
        .and_then(|id| snapshot.windows.iter().find(|window| window.id == id))
        .and_then(|window| window.app_id.clone())
        .filter(|app_id| app_id != APP_ID)
}

/// Whether `/proc/<pid>/stat` says the process is stopped (`T`) or stopped
/// under a tracer (`t`): it cannot answer anything until resumed. The state
/// is the first field after the parenthesised command name, which may itself
/// contain spaces or parentheses, so it is read after the last `)`.
///
/// This is the only hang Linux reports for an arbitrary app. A process that
/// is running but has stopped drawing looks healthy here, so an app can be
/// unresponsive without the mark; the list never marks one that is fine.
pub fn stat_says_stopped(stat: &str) -> bool {
    stat.rfind(')')
        .and_then(|end| stat[end + 1..].split_whitespace().next())
        .is_some_and(|state| matches!(state, "T" | "t"))
}

#[cfg(target_os = "linux")]
pub fn process_stopped(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| stat_says_stopped(&stat))
}

#[cfg(not(target_os = "linux"))]
pub fn process_stopped(_pid: u32) -> bool {
    false
}

pub fn confirmation_title(name: &str) -> String {
    format!("Do you want to force \u{201c}{name}\u{201d} to quit?")
}

pub const CONFIRMATION_MESSAGE: &str = "You will lose any unsaved changes.";

/// The list and its selection. The selection follows its app across
/// refreshes and never wraps, as in an AppKit table.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct List {
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
}

impl List {
    pub fn new(entries: Vec<Entry>, frontmost: Option<&str>) -> Self {
        let selected = frontmost
            .and_then(|app_id| entries.iter().position(|entry| entry.app_id == app_id))
            .or_else(|| (!entries.is_empty()).then_some(0));
        Self { entries, selected }
    }

    /// Replace the entries after an app starts or quits, keeping the
    /// selected app selected, or its old position when it went away.
    pub fn refresh(&mut self, entries: Vec<Entry>) {
        let previous = self.selected_entry().map(|entry| entry.app_id.clone());
        let old_index = self.selected;
        self.entries = entries;
        self.selected = previous
            .and_then(|app_id| self.entries.iter().position(|entry| entry.app_id == app_id))
            .or_else(|| {
                old_index
                    .filter(|_| !self.entries.is_empty())
                    .map(|index| index.min(self.entries.len() - 1))
            })
            .or_else(|| (!self.entries.is_empty()).then_some(0));
    }

    pub fn step(&mut self, forward: bool) {
        if self.entries.is_empty() {
            self.selected = None;
            return;
        }
        let last = self.entries.len() - 1;
        self.selected = Some(match (self.selected, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(index), true) => (index + 1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        });
    }

    pub fn select(&mut self, index: usize) {
        if index < self.entries.len() {
            self.selected = Some(index);
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.selected.and_then(|index| self.entries.get(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{FocusState, Window, WindowId, WorkspaceId};

    fn window(id: u64, app: &str, pid: Option<i32>) -> Window {
        Window {
            id: WindowId(id),
            title: None,
            app_id: Some(app.to_owned()),
            pid,
            workspace: Some(WorkspaceId(1)),
            focused: false,
            floating: true,
            urgent: false,
            focus_timestamp: None,
            layout: Default::default(),
        }
    }

    fn snapshot(windows: Vec<Window>, focused: Option<u64>) -> Snapshot {
        Snapshot {
            windows,
            focus: FocusState {
                window: focused.map(WindowId),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn name(app_id: &str) -> String {
        match app_id {
            "org.rmac.Files" => "Files".into(),
            "org.rmac.Terminal" => "Terminal".into(),
            "zed" => "Zed".into(),
            "firefox" => "Firefox".into(),
            other => other.into(),
        }
    }

    fn ids(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.app_id.as_str()).collect()
    }

    #[test]
    fn apps_are_alphabetical_with_files_last_and_shell_windows_left_out() {
        let snapshot = snapshot(
            vec![
                window(1, "org.rmac.Files", Some(40)),
                window(2, "zed", Some(10)),
                window(3, "org.rmac.Terminal", Some(20)),
                window(4, "firefox", Some(30)),
                window(5, "firefox", Some(31)),
                window(6, "firefox", Some(30)),
                window(7, "dev.rmac.TopBar", Some(50)),
                window(8, APP_ID, Some(60)),
                window(9, "org.rmac.Launcher", Some(70)),
            ],
            Some(2),
        );
        let entries = entries(&snapshot, name, |_| false);
        assert_eq!(
            ids(&entries),
            ["firefox", "org.rmac.Terminal", "zed", "org.rmac.Files"]
        );
        assert_eq!(entries[0].pids, [30, 31]);
        assert_eq!(frontmost_app(&snapshot).as_deref(), Some("zed"));
    }

    #[test]
    fn a_window_without_a_process_cannot_be_force_quit() {
        let snapshot = snapshot(vec![window(1, "zed", None), window(2, "a", Some(1))], None);
        let entries = entries(&snapshot, name, |_| false);
        assert!(entries.iter().all(|entry| !entry.can_force_quit()));
    }

    #[test]
    fn stopped_processes_are_marked_not_responding() {
        let snapshot = snapshot(
            vec![window(1, "zed", Some(10)), window(2, "firefox", Some(30))],
            None,
        );
        let entries = entries(&snapshot, name, |pid| pid == 10);
        assert!(!entries[0].not_responding);
        assert!(entries[1].not_responding);
    }

    #[test]
    fn proc_stat_state_is_read_after_the_last_parenthesis() {
        assert!(stat_says_stopped("1234 (zed) T 1 1234 1234 0 -1"));
        assert!(stat_says_stopped("1234 (my (odd) app) t 1 2"));
        assert!(!stat_says_stopped("1234 (T) S 1 2"));
        assert!(!stat_says_stopped("1234 (zed) R 1 2"));
        assert!(!stat_says_stopped(""));
        assert!(!stat_says_stopped("garbage"));
    }

    fn entry(app_id: &str) -> Entry {
        Entry {
            app_id: app_id.into(),
            name: app_id.into(),
            pids: vec![2],
            not_responding: false,
        }
    }

    #[test]
    fn selection_starts_on_the_frontmost_app_follows_it_and_never_wraps() {
        let mut list = List::new(vec![entry("a"), entry("b"), entry("c")], Some("b"));
        assert_eq!(list.selected, Some(1));
        list.step(true);
        list.step(true);
        assert_eq!(list.selected, Some(2));
        list.step(false);
        list.step(false);
        list.step(false);
        assert_eq!(list.selected, Some(0));

        list.select(2);
        list.refresh(vec![entry("0"), entry("a"), entry("b"), entry("c")]);
        assert_eq!(list.selected_entry().unwrap().app_id, "c");
        // The selected app quits: the selection stays in place.
        list.refresh(vec![entry("0"), entry("a"), entry("b")]);
        assert_eq!(list.selected_entry().unwrap().app_id, "b");
        list.refresh(Vec::new());
        assert_eq!(list.selected, None);
        list.refresh(vec![entry("a")]);
        assert_eq!(list.selected, Some(0));

        assert_eq!(List::new(vec![entry("a")], Some("gone")).selected, Some(0));
        assert_eq!(List::new(Vec::new(), None).selected, None);
    }

    #[test]
    fn confirmation_names_the_app_with_curly_quotes() {
        assert_eq!(
            confirmation_title("Zed"),
            "Do you want to force \u{201c}Zed\u{201d} to quit?"
        );
    }
}
