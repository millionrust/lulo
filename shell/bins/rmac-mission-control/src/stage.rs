//! Stage Manager (docs/decisions/0030-stage-manager.md).
//!
//! The focused app's windows stay on the Space ("the stage"); every other
//! app's windows wait on the hidden `rmac-stage` workspace as a "set", and
//! the sets are drawn as a strip down the left edge. Choosing a set (from
//! the strip, the Dock, ⌘Tab — anything that focuses one of its windows)
//! brings it back to the stage and sends the stage's set to the strip.
//!
//! This is the pure model: it turns compositor snapshots into the window
//! moves that keep that true. Nothing here runs unless Stage Manager is on.

use std::collections::{BTreeMap, BTreeSet};

use rmac_compositor::{
    is_hidden_workspace, Action, Snapshot, Window, WindowId, WorkspaceId, STAGE_WORKSPACE,
};

use crate::model::is_application;

/// The most sets the strip shows, as macOS does.
pub const MAX_SETS: usize = 6;

/// One app's windows waiting in the strip, the most recently used first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub key: String,
    pub app_id: Option<String>,
    pub windows: Vec<WindowId>,
}

#[derive(Clone, Debug, Default)]
pub struct Stage {
    pub enabled: bool,
    /// The Space the stage is on.
    home: Option<WorkspaceId>,
    /// Set keys, the most recently sent to the strip first.
    order: Vec<String>,
    /// Windows already asked to move, and where to: not asked again until
    /// they land (or go away).
    pending: BTreeMap<WindowId, WorkspaceId>,
    /// A set chosen in the strip, on its way to the stage: niri reports
    /// its windows landing before it reports the focus moving to them, and
    /// in between they must not be sent back to the strip.
    chosen: Option<String>,
}

/// A window's set: its app, or the window alone when it has no app ID.
fn set_key(window: &Window) -> String {
    window
        .app_id
        .clone()
        .unwrap_or_else(|| format!("window:{}", window.id.0))
}

fn workspace_name(snapshot: &Snapshot, workspace: Option<WorkspaceId>) -> Option<&str> {
    let workspace = workspace?;
    snapshot
        .workspaces
        .iter()
        .find(|candidate| candidate.id == workspace)
        .and_then(|candidate| candidate.name.as_deref())
}

fn stage_workspace(snapshot: &Snapshot) -> Option<WorkspaceId> {
    snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.name.as_deref() == Some(STAGE_WORKSPACE))
        .map(|workspace| workspace.id)
}

/// The Space a set comes back to when nothing better is known: the active
/// visible one on the focused output.
fn fallback_home(snapshot: &Snapshot) -> Option<WorkspaceId> {
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| !is_hidden_workspace(workspace.name.as_deref()))
        .find(|workspace| workspace.focused)
        .or_else(|| {
            snapshot
                .workspaces
                .iter()
                .filter(|workspace| !is_hidden_workspace(workspace.name.as_deref()))
                .find(|workspace| workspace.active)
        })
        .map(|workspace| workspace.id)
}

fn recency(window: &Window) -> (u64, u32) {
    window
        .focus_timestamp
        .map_or((0, 0), |time| (time.seconds, time.nanoseconds))
}

impl Stage {
    /// Forget moves that have landed or whose windows are gone.
    fn settle(&mut self, snapshot: &Snapshot) {
        self.pending.retain(|window, target| {
            snapshot
                .windows
                .iter()
                .find(|candidate| candidate.id == *window)
                .is_some_and(|candidate| candidate.workspace != Some(*target))
        });
    }

    fn move_window(&mut self, window: WindowId, workspace: WorkspaceId, follow: bool) -> Action {
        self.pending.insert(window, workspace);
        Action::MoveWindowToWorkspace {
            window,
            workspace,
            follow,
        }
    }

    /// The moves that keep only the focused app's windows on the stage.
    pub fn step(&mut self, snapshot: &Snapshot) -> Vec<Action> {
        self.settle(snapshot);
        if !self.enabled {
            return Vec::new();
        }
        let Some(stage) = stage_workspace(snapshot) else {
            return Vec::new();
        };
        let Some(focused) = snapshot
            .focus
            .window
            .and_then(|id| snapshot.windows.iter().find(|window| window.id == id))
        else {
            return Vec::new();
        };
        if !is_application(focused) {
            return Vec::new();
        }
        let key = set_key(focused);
        if self.chosen.as_ref() == Some(&key) {
            self.chosen = None;
        }
        let chosen = self.chosen.clone();
        let mut actions = Vec::new();
        if focused.workspace == Some(stage) {
            // A set was chosen (the strip, the Dock, ⌘Tab): bring it back.
            let Some(home) = self.home.or_else(|| fallback_home(snapshot)) else {
                return Vec::new();
            };
            let mut windows: Vec<&Window> = snapshot
                .windows
                .iter()
                .filter(|window| window.workspace == Some(stage) && set_key(window) == key)
                .filter(|window| !self.pending.contains_key(&window.id))
                .collect();
            // The focused one last, so it ends up on top with the focus.
            windows.sort_by_key(|window| window.id == focused.id);
            let ids: Vec<(WindowId, bool)> = windows
                .iter()
                .map(|window| (window.id, window.id == focused.id))
                .collect();
            for (window, follow) in ids {
                actions.push(self.move_window(window, home, follow));
            }
            self.order.retain(|candidate| *candidate != key);
            return actions;
        }
        if is_hidden_workspace(workspace_name(snapshot, focused.workspace)) {
            return Vec::new();
        }
        let Some(home) = focused.workspace else {
            return Vec::new();
        };
        self.home = Some(home);
        self.order.retain(|candidate| *candidate != key);
        let others: Vec<(WindowId, String)> = snapshot
            .windows
            .iter()
            .filter(|window| window.workspace == Some(home))
            .filter(|window| is_application(window) && set_key(window) != key)
            .filter(|window| chosen.as_ref() != Some(&set_key(window)))
            .filter(|window| !self.pending.contains_key(&window.id))
            .map(|window| (window.id, set_key(window)))
            .collect();
        for (window, set) in others {
            if !self.order.contains(&set) {
                self.order.insert(0, set);
            }
            actions.push(self.move_window(window, stage, false));
        }
        actions
    }

    /// A set chosen in the strip: its windows come straight back to the
    /// stage, the most recent last and focused, without niri first showing
    /// the stage workspace. The set on the stage then goes to the strip
    /// when [`Stage::step`] sees the focus land.
    pub fn choose(&mut self, snapshot: &Snapshot, set: &Set) -> Vec<Action> {
        self.settle(snapshot);
        let Some(home) = self.home.or_else(|| fallback_home(snapshot)) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        for window in set.windows.iter().rev() {
            if self.pending.contains_key(window) {
                continue;
            }
            let focused = Some(window) == set.windows.first();
            actions.push(self.move_window(*window, home, focused));
        }
        if let Some(window) = set.windows.first() {
            actions.push(Action::FocusWindow { window: *window });
        }
        self.order.retain(|candidate| *candidate != set.key);
        self.chosen = Some(set.key.clone());
        actions
    }

    /// A wallpaper click with Stage Manager on: the set on the stage joins
    /// the strip and the desktop shows, as on macOS.
    pub fn clear(&mut self, snapshot: &Snapshot) -> Vec<Action> {
        self.settle(snapshot);
        if !self.enabled {
            return Vec::new();
        }
        let Some(stage) = stage_workspace(snapshot) else {
            return Vec::new();
        };
        let Some(home) = self.home.or_else(|| fallback_home(snapshot)) else {
            return Vec::new();
        };
        let windows: Vec<(WindowId, String)> = snapshot
            .windows
            .iter()
            .filter(|window| window.workspace == Some(home) && is_application(window))
            .filter(|window| !self.pending.contains_key(&window.id))
            .map(|window| (window.id, set_key(window)))
            .collect();
        let mut actions = Vec::new();
        for (window, set) in windows {
            if !self.order.contains(&set) {
                self.order.insert(0, set);
            }
            actions.push(self.move_window(window, stage, false));
        }
        actions
    }

    /// Stage Manager turned off: every set comes back to the stage's Space.
    pub fn release(&mut self, snapshot: &Snapshot) -> Vec<Action> {
        self.settle(snapshot);
        let Some(stage) = stage_workspace(snapshot) else {
            return Vec::new();
        };
        let Some(home) = self.home.or_else(|| fallback_home(snapshot)) else {
            return Vec::new();
        };
        let windows: Vec<WindowId> = snapshot
            .windows
            .iter()
            .filter(|window| window.workspace == Some(stage))
            .map(|window| window.id)
            .collect();
        self.order.clear();
        windows
            .into_iter()
            .map(|window| self.move_window(window, home, false))
            .collect()
    }

    /// The strip: the sets waiting on the stage workspace, the one sent
    /// there most recently first, at most [`MAX_SETS`].
    pub fn sets(&self, snapshot: &Snapshot) -> Vec<Set> {
        let Some(stage) = stage_workspace(snapshot) else {
            return Vec::new();
        };
        let mut sets: BTreeMap<String, Vec<&Window>> = BTreeMap::new();
        for window in snapshot
            .windows
            .iter()
            .filter(|window| window.workspace == Some(stage))
        {
            sets.entry(set_key(window)).or_default().push(window);
        }
        let known: BTreeSet<&String> = self.order.iter().collect();
        let mut keys: Vec<String> = self
            .order
            .iter()
            .filter(|key| sets.contains_key(*key))
            .cloned()
            .collect();
        keys.extend(sets.keys().filter(|key| !known.contains(key)).cloned());
        keys.into_iter()
            .take(MAX_SETS)
            .filter_map(|key| {
                let mut windows = sets.remove(&key)?;
                windows.sort_by_key(|window| std::cmp::Reverse(recency(window)));
                Some(Set {
                    app_id: windows[0].app_id.clone(),
                    windows: windows.iter().map(|window| window.id).collect(),
                    key,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{FocusState, Timestamp, WindowLayout, Workspace};

    fn workspace(id: u64, name: Option<&str>, active: bool) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: name.map(str::to_owned),
            output: Some(rmac_compositor::OutputId::from("eDP-1")),
            urgent: false,
            active,
            focused: active,
            active_window: None,
        }
    }

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
            layout: WindowLayout::default(),
        }
    }

    fn snapshot(windows: Vec<Window>, focused: u64) -> Snapshot {
        Snapshot {
            workspaces: vec![
                workspace(1, Some("Desktop"), true),
                workspace(2, Some(rmac_compositor::PARKING_WORKSPACE), false),
                workspace(3, Some(STAGE_WORKSPACE), false),
            ],
            windows,
            focus: FocusState {
                window: Some(WindowId(focused)),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn moves(actions: &[Action]) -> Vec<(u64, u64, bool)> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::MoveWindowToWorkspace {
                    window,
                    workspace,
                    follow,
                } => Some((window.0, workspace.0, *follow)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn off_it_moves_nothing() {
        let mut stage = Stage::default();
        let snapshot = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 2),
                window(2, "org.rmac.Files", 1, 1),
            ],
            1,
        );
        assert!(stage.step(&snapshot).is_empty());
    }

    #[test]
    fn the_focused_app_keeps_the_stage_and_the_others_go_to_the_strip() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let before = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 3),
                window(2, "org.rmac.Notes", 1, 2),
                window(3, "org.rmac.Files", 1, 1),
                // Minimised windows stay minimised.
                window(4, "org.rmac.Terminal", 2, 1),
            ],
            1,
        );
        assert_eq!(moves(&stage.step(&before)), [(3, 3, false)]);
        // Asked once: the same snapshot again moves nothing.
        assert!(stage.step(&before).is_empty());

        let after = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 3),
                window(2, "org.rmac.Notes", 1, 2),
                window(3, "org.rmac.Files", 3, 1),
            ],
            1,
        );
        assert!(stage.step(&after).is_empty());
        let sets = stage.sets(&after);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].app_id.as_deref(), Some("org.rmac.Files"));
    }

    #[test]
    fn choosing_a_set_brings_it_back_and_sends_the_stage_to_the_strip() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let staged = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 3),
                window(3, "org.rmac.Files", 3, 1),
            ],
            1,
        );
        stage.step(&staged);
        // Files was focused from the strip: niri focused it where it waits.
        let chosen = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 3),
                window(3, "org.rmac.Files", 3, 4),
            ],
            3,
        );
        assert_eq!(moves(&stage.step(&chosen)), [(3, 1, true)]);
        // Back on the Space and focused, Files takes the stage from Notes.
        let swapped = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 3),
                window(3, "org.rmac.Files", 1, 4),
            ],
            3,
        );
        assert_eq!(moves(&stage.step(&swapped)), [(1, 3, false)]);
    }

    #[test]
    fn a_set_chosen_in_the_strip_comes_straight_back_focused() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let staged = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 9),
                window(3, "org.rmac.Files", 3, 1),
                window(4, "org.rmac.Files", 3, 5),
            ],
            1,
        );
        stage.step(&staged);
        let sets = stage.sets(&staged);
        assert_eq!(sets[0].windows, [WindowId(4), WindowId(3)]);
        let actions = stage.choose(&staged, &sets[0]);
        assert_eq!(moves(&actions), [(3, 1, false), (4, 1, true)]);
        assert_eq!(
            actions.last(),
            Some(&Action::FocusWindow {
                window: WindowId(4)
            })
        );
    }

    #[test]
    fn a_chosen_set_landing_before_the_focus_stays_on_the_stage() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let staged = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 9),
                window(3, "org.rmac.Files", 3, 1),
            ],
            1,
        );
        stage.step(&staged);
        let sets = stage.sets(&staged);
        stage.choose(&staged, &sets[0]);
        // Files has landed; the focus is still on Notes.
        let landed = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 9),
                window(3, "org.rmac.Files", 1, 1),
            ],
            1,
        );
        assert!(stage.step(&landed).is_empty());
        // Then the focus arrives, and Notes goes to the strip.
        let focused = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 9),
                window(3, "org.rmac.Files", 1, 10),
            ],
            3,
        );
        assert_eq!(moves(&stage.step(&focused)), [(1, 3, false)]);
    }

    #[test]
    fn a_wallpaper_click_sends_the_stage_to_the_strip() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let staged = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 9),
                window(3, "org.rmac.Files", 3, 1),
            ],
            1,
        );
        stage.step(&staged);
        assert_eq!(moves(&stage.clear(&staged)), [(1, 3, false)]);
        stage.enabled = false;
        assert!(stage.clear(&staged).is_empty());
    }

    #[test]
    fn turning_it_off_brings_every_set_back() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let staged = snapshot(
            vec![
                window(1, "org.rmac.Notes", 1, 3),
                window(3, "org.rmac.Files", 3, 1),
                window(5, "org.rmac.Clock", 3, 2),
            ],
            1,
        );
        stage.step(&staged);
        stage.enabled = false;
        let mut back = moves(&stage.release(&staged));
        back.sort();
        assert_eq!(back, [(3, 1, false), (5, 1, false)]);
    }

    #[test]
    fn the_strip_lists_sets_most_recent_first_and_at_most_six() {
        let mut stage = Stage {
            enabled: true,
            ..Default::default()
        };
        let mut windows = vec![window(1, "org.rmac.Notes", 1, 100)];
        for index in 0..8u64 {
            windows.push(window(10 + index, &format!("app.{index}"), 1, index));
        }
        let first = snapshot(windows.clone(), 1);
        stage.step(&first);
        for window in windows.iter_mut().skip(1) {
            window.workspace = Some(WorkspaceId(3));
        }
        let staged = snapshot(windows, 1);
        assert_eq!(stage.sets(&staged).len(), MAX_SETS);
    }
}
