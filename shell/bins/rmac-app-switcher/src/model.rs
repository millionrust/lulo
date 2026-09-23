//! Per-application ⌘Tab model: most-recently-used order, selection, geometry,
//! and the compositor actions that activate, hide, or quit an application.
//!
//! Nothing here touches GPUI, sockets, or niri, so the behavior is tested on
//! every platform.

use rmac_compositor::{window_is_parked, Action, Snapshot, Timestamp, WindowId, WorkspaceId};

/// Shell helper windows that are never applications in the switcher.
const EXCLUDED_APP_IDS: [&str; 3] = [
    "org.rmac.Launcher",
    "org.rmac.QuickSettings",
    "org.rmac.NotificationCenter",
];
const EXCLUDED_APP_PREFIX: &str = "dev.rmac.";
const MAX_REMEMBERED_APPS: usize = 256;

/// One running application: every window that carries its app id.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningApp {
    pub app_id: String,
    /// Most recently focused first.
    pub windows: Vec<WindowId>,
    /// Every window is parked (the app was hidden with ⌘H or Hide).
    pub hidden: bool,
}

pub fn is_switchable(app_id: &str) -> bool {
    !app_id.is_empty()
        && !app_id.starts_with(EXCLUDED_APP_PREFIX)
        && !EXCLUDED_APP_IDS.contains(&app_id)
}

fn stamp(timestamp: Option<Timestamp>) -> (u64, u32) {
    timestamp.map_or((0, 0), |time| (time.seconds, time.nanoseconds))
}

/// Application activation order learned from focus changes, most recent
/// first. The compositor's per-window focus timestamps order anything this
/// process has not yet seen focused.
#[derive(Clone, Debug, Default)]
pub struct Recency {
    order: Vec<String>,
}

impl Recency {
    pub fn observe_focus(&mut self, app_id: &str) {
        if !is_switchable(app_id) {
            return;
        }
        self.order.retain(|known| known != app_id);
        self.order.insert(0, app_id.to_owned());
        self.order.truncate(MAX_REMEMBERED_APPS);
    }

    pub fn forget(&mut self, app_id: &str) {
        self.order.retain(|known| known != app_id);
    }

    /// Running applications in ⌘Tab order: the focused application first,
    /// then the ones this session activated, then everything else by the
    /// compositor's most recent focus time.
    pub fn applications(&self, snapshot: &Snapshot) -> Vec<RunningApp> {
        let mut apps: Vec<(RunningApp, (u64, u32))> = Vec::new();
        let mut windows = snapshot.windows.iter().collect::<Vec<_>>();
        windows.sort_by(|left, right| {
            stamp(right.focus_timestamp)
                .cmp(&stamp(left.focus_timestamp))
                .then(left.id.cmp(&right.id))
        });
        for window in windows {
            let Some(app_id) = window.app_id.as_deref().filter(|id| is_switchable(id)) else {
                continue;
            };
            let parked = window_is_parked(snapshot, window);
            match apps.iter_mut().find(|(app, _)| app.app_id == app_id) {
                Some((app, _)) => {
                    app.windows.push(window.id);
                    app.hidden &= parked;
                }
                None => apps.push((
                    RunningApp {
                        app_id: app_id.to_owned(),
                        windows: vec![window.id],
                        hidden: parked,
                    },
                    stamp(window.focus_timestamp),
                )),
            }
        }

        let focused_app = snapshot
            .focus
            .window
            .and_then(|id| snapshot.windows.iter().find(|window| window.id == id))
            .and_then(|window| window.app_id.clone());
        let rank = |app: &RunningApp| -> (u8, usize) {
            if focused_app.as_deref() == Some(app.app_id.as_str()) {
                return (0, 0);
            }
            match self.order.iter().position(|known| *known == app.app_id) {
                Some(position) => (1, position),
                None => (2, 0),
            }
        };
        // Stable: equal ranks keep the focus-time order built above.
        apps.sort_by(|(left, left_time), (right, right_time)| {
            rank(left)
                .cmp(&rank(right))
                .then(right_time.cmp(left_time))
        });
        apps.into_iter().map(|(app, _)| app).collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Next,
    Previous,
    Cancel,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "next" => Some(Self::Next),
            "previous" => Some(Self::Previous),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Cancel => "cancel",
        }
    }
}

/// One ⌘Tab hold: the applications captured when it opened and the
/// selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub apps: Vec<RunningApp>,
    pub selected: usize,
}

impl Session {
    /// ⌘Tab starts on the previous application; ⌘⇧Tab starts on the least
    /// recent one. A single application stays selected.
    pub fn open(apps: Vec<RunningApp>, backwards: bool) -> Option<Self> {
        if apps.is_empty() {
            return None;
        }
        let selected = match (apps.len(), backwards) {
            (1, _) => 0,
            (count, true) => count - 1,
            (_, false) => 1,
        };
        Some(Self { apps, selected })
    }

    pub fn step(&mut self, forward: bool) {
        let count = self.apps.len();
        if count == 0 {
            return;
        }
        self.selected = if forward {
            (self.selected + 1) % count
        } else {
            (self.selected + count - 1) % count
        };
    }

    pub fn select(&mut self, index: usize) {
        if index < self.apps.len() {
            self.selected = index;
        }
    }

    pub fn selected_app(&self) -> Option<&RunningApp> {
        self.apps.get(self.selected)
    }

    /// Drop a quit application and keep the selection on its neighbour.
    /// Returns false when nothing is left to show.
    pub fn remove(&mut self, app_id: &str) -> bool {
        if let Some(index) = self.apps.iter().position(|app| app.app_id == app_id) {
            self.apps.remove(index);
            if self.selected > index || self.selected >= self.apps.len() {
                self.selected = self.selected.saturating_sub(1);
            }
        }
        !self.apps.is_empty()
    }

    pub fn mark_hidden(&mut self, app_id: &str) {
        if let Some(app) = self.apps.iter_mut().find(|app| app.app_id == app_id) {
            app.hidden = true;
        }
    }
}

/// Panel geometry in logical points, measured on macOS 26 (see
/// design-lab/switcher-osd.html) and scaled down uniformly when the row would
/// not fit the display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    pub padding: f32,
    pub icon: f32,
    pub gap: f32,
    pub radius: f32,
    pub plate_inset: f32,
    pub plate_radius: f32,
    /// Top of the 16 pt name line box, measured from the panel top.
    pub label_top: f32,
}

pub const PADDING: f32 = 24.0;
pub const ICON: f32 = 128.0;
pub const GAP: f32 = 6.0;
pub const HEIGHT: f32 = 176.0;
pub const RADIUS: f32 = 56.0;
pub const PLATE_INSET: f32 = 4.0;
pub const PLATE_RADIUS: f32 = 32.0;
pub const LABEL_LINE: f32 = 16.0;
/// The panel never scales below half size; the row clips beyond that.
const MIN_SCALE: f32 = 0.5;
/// Space kept between the panel and the display edges.
pub const SCREEN_MARGIN: f32 = 40.0;

pub fn natural_width(count: usize) -> f32 {
    let count = count.max(1) as f32;
    2.0 * PADDING + count * ICON + (count - 1.0) * GAP
}

pub fn layout(count: usize, display_width: f32) -> Layout {
    let natural = natural_width(count);
    let available = (display_width - 2.0 * SCREEN_MARGIN).max(1.0);
    let scale = if natural > available {
        (available / natural).max(MIN_SCALE)
    } else {
        1.0
    };
    let height = HEIGHT * scale;
    Layout {
        scale,
        width: (natural * scale).round(),
        height: height.round(),
        padding: PADDING * scale,
        icon: ICON * scale,
        gap: GAP * scale,
        radius: RADIUS * scale,
        plate_inset: PLATE_INSET * scale,
        plate_radius: PLATE_RADIUS * scale,
        // Natural: the name's line box starts right under the 128 frame
        // (152), putting its baseline 12 above the bottom edge.
        label_top: ((PADDING + ICON) * scale).min(height - LABEL_LINE),
    }
}

/// Where a parked window returns to: its recorded origin, else the
/// workspace the user is on now.
pub fn restore_workspace(
    recorded: Option<WorkspaceId>,
    snapshot: &Snapshot,
) -> Option<WorkspaceId> {
    recorded.or(snapshot.focus.workspace).or_else(|| {
        snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.focused || workspace.active)
            .map(|workspace| workspace.id)
    })
}

/// Bring an application forward the way macOS does: unhide it if hidden,
/// then raise every window that shares a workspace with its most recent
/// window, finishing on that window so it takes keyboard focus.
///
/// `origin` resolves a parked window's recorded workspace.
pub fn activation_actions(
    snapshot: &Snapshot,
    app: &RunningApp,
    origin: impl Fn(WindowId) -> Option<WorkspaceId>,
) -> Vec<Action> {
    let live = |id: WindowId| snapshot.windows.iter().find(|window| window.id == id);
    let windows = app
        .windows
        .iter()
        .filter_map(|id| live(*id))
        .collect::<Vec<_>>();
    let (parked, visible): (Vec<_>, Vec<_>) = windows
        .into_iter()
        .partition(|window| window_is_parked(snapshot, window));

    if visible.is_empty() {
        // A hidden app: restore every window, most recent last so it ends
        // up focused.
        return parked
            .iter()
            .rev()
            .filter_map(|window| {
                restore_workspace(origin(window.id), snapshot).map(|workspace| {
                    Action::RestoreWindow {
                        window: window.id,
                        workspace,
                    }
                })
            })
            .collect();
    }

    let front = visible[0];
    visible
        .iter()
        .rev()
        .filter(|window| window.workspace == front.workspace)
        .map(|window| Action::FocusWindow { window: window.id })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{FocusState, Window, Workspace, PARKING_WORKSPACE};

    fn window(id: u64, app: &str, workspace: u64, focused_at: u64) -> Window {
        Window {
            id: WindowId(id),
            title: None,
            app_id: Some(app.to_owned()),
            pid: None,
            workspace: Some(WorkspaceId(workspace)),
            focused: false,
            floating: true,
            urgent: false,
            focus_timestamp: Some(Timestamp {
                seconds: focused_at,
                nanoseconds: 0,
            }),
            layout: Default::default(),
        }
    }

    fn workspace(id: u64, name: Option<&str>) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: name.map(str::to_owned),
            output: None,
            urgent: false,
            active: id == 1,
            focused: id == 1,
            active_window: None,
        }
    }

    fn snapshot(windows: Vec<Window>, focused: Option<u64>) -> Snapshot {
        Snapshot {
            workspaces: vec![
                workspace(1, Some("Desktop")),
                workspace(2, None),
                workspace(9, Some(PARKING_WORKSPACE)),
            ],
            windows,
            focus: FocusState {
                window: focused.map(WindowId),
                workspace: Some(WorkspaceId(1)),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn ids(apps: &[RunningApp]) -> Vec<&str> {
        apps.iter().map(|app| app.app_id.as_str()).collect()
    }

    #[test]
    fn groups_windows_per_application_in_recent_use_order() {
        let snapshot = snapshot(
            vec![
                window(1, "org.rmac.Files", 1, 10),
                window(2, "zed", 1, 40),
                window(3, "firefox", 1, 30),
                window(4, "org.rmac.Files", 2, 20),
                window(5, "dev.rmac.TopBar", 1, 50),
                window(6, "org.rmac.Launcher", 1, 60),
            ],
            Some(2),
        );
        let apps = Recency::default().applications(&snapshot);
        assert_eq!(ids(&apps), ["zed", "firefox", "org.rmac.Files"]);
        assert_eq!(apps[2].windows, [WindowId(4), WindowId(1)]);
    }

    #[test]
    fn observed_activations_outrank_focus_times_but_not_the_focused_app() {
        let snapshot = snapshot(
            vec![
                window(1, "a", 1, 10),
                window(2, "b", 1, 30),
                window(3, "c", 1, 20),
            ],
            Some(2),
        );
        let mut recency = Recency::default();
        recency.observe_focus("a");
        recency.observe_focus("b");
        assert_eq!(ids(&recency.applications(&snapshot)), ["b", "a", "c"]);
        recency.forget("a");
        assert_eq!(ids(&recency.applications(&snapshot)), ["b", "c", "a"]);
    }

    #[test]
    fn hidden_applications_stay_listed_and_are_marked() {
        let snapshot = snapshot(
            vec![window(1, "a", 1, 10), window(2, "b", 9, 30)],
            Some(1),
        );
        let apps = Recency::default().applications(&snapshot);
        assert_eq!(ids(&apps), ["a", "b"]);
        assert!(apps[1].hidden);
        assert!(!apps[0].hidden);
    }

    #[test]
    fn a_session_starts_on_the_previous_app_and_wraps() {
        let app = |id: &str| RunningApp {
            app_id: id.into(),
            windows: vec![],
            hidden: false,
        };
        let mut session = Session::open(vec![app("a"), app("b"), app("c")], false).unwrap();
        assert_eq!(session.selected, 1);
        session.step(true);
        session.step(true);
        assert_eq!(session.selected, 0);
        session.step(false);
        assert_eq!(session.selected, 2);
        assert_eq!(
            Session::open(vec![app("a"), app("b"), app("c")], true)
                .unwrap()
                .selected,
            2
        );
        assert_eq!(Session::open(vec![app("a")], false).unwrap().selected, 0);
        assert!(Session::open(vec![], false).is_none());

        session.select(2);
        assert!(session.remove("c"));
        assert_eq!(session.selected_app().unwrap().app_id, "b");
        session.select(0);
        assert!(session.remove("b"));
        assert_eq!(session.selected_app().unwrap().app_id, "a");
        assert!(!session.remove("a"));
    }

    #[test]
    fn commands_are_an_exact_allow_list() {
        for command in [Command::Next, Command::Previous, Command::Cancel] {
            assert_eq!(Command::parse(command.as_str()), Some(command));
        }
        assert_eq!(Command::parse("next;quit"), None);
        assert_eq!(Command::parse(""), None);
    }

    #[test]
    fn layout_matches_the_mac_and_scales_to_fit() {
        let five = layout(5, 1470.0);
        assert_eq!(five.scale, 1.0);
        assert_eq!(five.width, 712.0);
        assert_eq!(five.height, 176.0);
        assert_eq!(five.label_top, 152.0);

        let many = layout(12, 1280.0);
        assert!(many.scale < 1.0);
        assert!(many.width <= 1280.0 - 2.0 * SCREEN_MARGIN);
        assert!(many.label_top + LABEL_LINE <= many.height + 0.01);
    }

    #[test]
    fn activation_raises_the_front_workspace_and_focuses_the_latest_window_last() {
        let snapshot = snapshot(
            vec![
                window(1, "a", 1, 30),
                window(2, "a", 1, 20),
                window(3, "a", 2, 10),
            ],
            None,
        );
        let app = Recency::default().applications(&snapshot).remove(0);
        assert_eq!(
            activation_actions(&snapshot, &app, |_| None),
            [
                Action::FocusWindow {
                    window: WindowId(2)
                },
                Action::FocusWindow {
                    window: WindowId(1)
                },
            ]
        );
    }

    #[test]
    fn activating_a_hidden_app_restores_its_windows() {
        let snapshot = snapshot(
            vec![window(1, "a", 9, 30), window(2, "a", 9, 20)],
            None,
        );
        let app = Recency::default().applications(&snapshot).remove(0);
        let actions = activation_actions(&snapshot, &app, |window| {
            (window == WindowId(1)).then_some(WorkspaceId(2))
        });
        assert_eq!(
            actions,
            [
                Action::RestoreWindow {
                    window: WindowId(2),
                    workspace: WorkspaceId(1),
                },
                Action::RestoreWindow {
                    window: WindowId(1),
                    workspace: WorkspaceId(2),
                },
            ]
        );
    }
}
