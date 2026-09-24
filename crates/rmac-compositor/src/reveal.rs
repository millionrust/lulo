//! Click wallpaper to reveal desktop (Desktop & Dock › "Click wallpaper to
//! reveal desktop"), measured on macOS 26.2 at 1470 × 956 pt
//! (design-lab/reveal-desktop.html):
//!
//! - Every window slides along the ray from the centre of the screen below
//!   the menu bar through the window's own centre.
//! - It stops at the first edge that ray reaches, leaving a 12 pt sliver
//!   inside. The top edge is the bottom of the menu bar. The bottom edge is
//!   the screen's own, under the Dock.
//! - Clicking the wallpaper again, clicking a sliver or activating any app
//!   brings every window back to where it was.
//!
//! This module holds only the geometry and the rules over a [`Snapshot`].
//! The Mission Control service sends the actions and follows focus. Nothing
//! is persisted, so [`recover`] is how a restarted service pulls back windows
//! that an earlier run left at an edge.

use crate::{
    window_is_parked, Action, Distance, LogicalPoint, LogicalRect, Output, Snapshot, Window,
    WindowId, WorkspaceId,
};

/// The sliver of each window the Mac leaves on screen, in points.
pub const MAC_SLIVER: f64 = 12.0;

/// Compositor rounding: positions snap to physical pixels, so a window
/// pushed to a `sliver` can show up to a pixel more.
const SLACK: f64 = 1.0;

/// The screen edge a window was pushed towards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// How far one window moves to reveal the desktop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Push {
    pub edge: Edge,
    pub dx: f64,
    pub dy: f64,
}

/// Where `window` goes: along the ray from the centre of `area` through the
/// window's centre, until only `sliver` of it (or all of it, if it is
/// narrower) stays inside `area` at the first edge the ray reaches.
///
/// Returns `None` when the window already shows no more than the sliver.
/// A window centred exactly on the area moves up (S: not measured).
pub fn push_aside(area: LogicalRect, window: LogicalRect, sliver: f64) -> Option<Push> {
    let mut dx = window.x + window.width / 2.0 - (area.x + area.width / 2.0);
    let mut dy = window.y + window.height / 2.0 - (area.y + area.height / 2.0);
    if !(dx.is_finite() && dy.is_finite() && sliver.is_finite()) {
        return None;
    }
    if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
        dx = 0.0;
        dy = -1.0;
    }
    let keep_x = sliver.min(window.width);
    let keep_y = sliver.min(window.height);
    let right = area.x + area.width;
    let bottom = area.y + area.height;
    let candidates = [
        (dx > 0.0).then(|| ((right - keep_x - window.x) / dx, Edge::Right)),
        (dx < 0.0).then(|| ((area.x + keep_x - window.x - window.width) / dx, Edge::Left)),
        (dy > 0.0).then(|| ((bottom - keep_y - window.y) / dy, Edge::Bottom)),
        (dy < 0.0).then(|| ((area.y + keep_y - window.y - window.height) / dy, Edge::Top)),
    ];
    let (t, edge) = candidates
        .into_iter()
        .flatten()
        .filter(|(t, _)| t.is_finite())
        .min_by(|left, right| left.0.total_cmp(&right.0))?;
    (t > 0.0).then_some(Push {
        edge,
        dx: dx * t,
        dy: dy * t,
    })
}

/// The area windows are pushed within: `output` below the menu bar, in
/// output-local logical coordinates (the space of `tile_position_in_view`).
pub fn output_area(output: &Output, top_inset: f64) -> Option<LogicalRect> {
    let logical = output.logical.as_ref()?;
    let height = logical.size.height - top_inset;
    (logical.size.is_valid() && height > 0.0).then_some(LogicalRect {
        x: 0.0,
        y: top_inset,
        width: logical.size.width,
        height,
    })
}

/// One window that was pushed aside.
#[derive(Clone, Debug, PartialEq)]
pub struct Moved {
    pub window: WindowId,
    /// Where the window was, output-local.
    pub origin: LogicalPoint,
    /// The move that was asked for.
    pub dx: f64,
    pub dy: f64,
}

/// A revealed desktop: what moved, and what brings it back.
#[derive(Clone, Debug, PartialEq)]
pub struct Revealed {
    pub moved: Vec<Moved>,
    /// The Spaces on screen when the desktop was revealed. Leaving one
    /// brings the windows back.
    pub workspaces: Vec<WorkspaceId>,
    /// The focused window last seen. Focusing a window after this (clicking
    /// a sliver, activating an app, opening an item) brings them back.
    pub focus: Option<WindowId>,
}

impl Revealed {
    /// Follow the compositor. Returns whether the windows must come back now.
    pub fn should_restore(&mut self, snapshot: &Snapshot) -> bool {
        let focus = snapshot.focus.window;
        let focused_a_window = focus.is_some() && focus != self.focus;
        self.focus = focus;
        let left_space = self.workspaces.iter().any(|id| {
            !snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.id == *id && workspace.active)
        });
        focused_a_window || left_space
    }
}

/// A floating, visible window's area and output-local frame.
fn placement(
    snapshot: &Snapshot,
    window: &Window,
    top_inset: f64,
) -> Option<(LogicalRect, LogicalRect)> {
    if !window.floating || window_is_parked(snapshot, window) {
        return None;
    }
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| Some(workspace.id) == window.workspace)?;
    if !workspace.active {
        return None;
    }
    let output = snapshot
        .outputs
        .iter()
        .find(|output| Some(&output.id) == workspace.output.as_ref())?;
    let area = output_area(output, top_inset)?;
    let tile = window.layout.tile_position_in_view?;
    let size = window.layout.tile_size;
    if !(size.is_valid() && size.width > 0.0 && size.height > 0.0) {
        return None;
    }
    Some((
        area,
        LogicalRect {
            x: tile.x,
            y: tile.y,
            width: size.width,
            height: size.height,
        },
    ))
}

fn move_by(window: WindowId, dx: f64, dy: f64) -> Action {
    Action::MoveWindowBy {
        window,
        dx: Distance(dx),
        dy: Distance(dy),
    }
}

/// Push every floating window on the Spaces now showing towards its edge.
/// `top_inset` is the menu bar's height; `sliver` is how much of each window
/// stays on screen. `None` when there is nothing to move.
pub fn reveal(snapshot: &Snapshot, top_inset: f64, sliver: f64) -> Option<(Revealed, Vec<Action>)> {
    let mut moved = Vec::new();
    let mut actions = Vec::new();
    for window in &snapshot.windows {
        let Some((area, frame)) = placement(snapshot, window, top_inset) else {
            continue;
        };
        let Some(push) = push_aside(area, frame, sliver) else {
            continue;
        };
        moved.push(Moved {
            window: window.id,
            origin: LogicalPoint {
                x: frame.x,
                y: frame.y,
            },
            dx: push.dx,
            dy: push.dy,
        });
        actions.push(move_by(window.id, push.dx, push.dy));
    }
    if moved.is_empty() {
        return None;
    }
    let workspaces = snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.active)
        .map(|workspace| workspace.id)
        .collect();
    Some((
        Revealed {
            moved,
            workspaces,
            focus: snapshot.focus.window,
        },
        actions,
    ))
}

/// Bring every pushed window that still exists back to where it was. The
/// move is measured from where the compositor now reports the window, so a
/// move it cut short is undone exactly. A window with no position (on a
/// hidden Space) undoes the move that was asked for.
pub fn restore(snapshot: &Snapshot, revealed: &Revealed) -> Vec<Action> {
    revealed
        .moved
        .iter()
        .filter_map(|moved| {
            let window = snapshot
                .windows
                .iter()
                .find(|window| window.id == moved.window)?;
            let (dx, dy) = match window.layout.tile_position_in_view {
                Some(now) => (moved.origin.x - now.x, moved.origin.y - now.y),
                None => (-moved.dx, -moved.dy),
            };
            (dx.abs() >= 0.5 || dy.abs() >= 0.5).then(|| move_by(window.id, dx, dy))
        })
        .collect()
}

/// Windows left at an edge by a service that stopped while the desktop was
/// revealed: any floating window on screen that shows no more than `sliver`
/// of itself inside its area comes fully back inside (or to the area's
/// top-left corner when it is larger than the area).
pub fn recover(snapshot: &Snapshot, top_inset: f64, sliver: f64) -> Vec<Action> {
    let limit = sliver + SLACK;
    snapshot
        .windows
        .iter()
        .filter_map(|window| {
            let (area, frame) = placement(snapshot, window, top_inset)?;
            let right = area.x + area.width;
            let bottom = area.y + area.height;
            let shown_x = (frame.x + frame.width).min(right) - frame.x.max(area.x);
            let shown_y = (frame.y + frame.height).min(bottom) - frame.y.max(area.y);
            let stranded = (frame.width > limit && shown_x <= limit)
                || (frame.height > limit && shown_y <= limit);
            if !stranded {
                return None;
            }
            let x = frame.x.min(right - frame.width).max(area.x);
            let y = frame.y.min(bottom - frame.height).max(area.y);
            Some(move_by(window.id, x - frame.x, y - frame.y))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FocusState, LogicalOutput, LogicalSize, OutputId, WindowLayout, Workspace,
        PARKING_WORKSPACE,
    };

    /// The owner's Mac: 1470 × 956 with a 33 pt menu bar.
    const MAC_AREA: LogicalRect = LogicalRect {
        x: 0.0,
        y: 33.0,
        width: 1470.0,
        height: 923.0,
    };

    fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
        LogicalRect {
            x,
            y,
            width,
            height,
        }
    }

    fn landed(frame: LogicalRect, push: Push) -> (f64, f64) {
        (frame.x + push.dx, frame.y + push.dy)
    }

    fn close(actual: (f64, f64), expected: (f64, f64)) -> bool {
        (actual.0 - expected.0).abs() <= 1.0 && (actual.1 - expected.1).abs() <= 1.0
    }

    #[test]
    fn windows_land_where_the_mac_put_them() {
        // Frames before and after one wallpaper click on macOS 26.2, read
        // from the window server (design-lab/reveal-desktop.html).
        let cases = [
            // Finder, left of centre: to the left edge, a little up.
            (
                rect(400.0, 300.0, 600.0, 380.0),
                Edge::Left,
                (-588.0, 173.0),
            ),
            // TextEdit, top left: to the left edge, well up.
            (
                rect(100.0, 100.0, 500.0, 350.0),
                Edge::Left,
                (-488.0, -235.0),
            ),
            // TextEdit, bottom right: to the right edge, down.
            (
                rect(850.0, 480.0, 500.0, 350.0),
                Edge::Right,
                (1458.0, 748.0),
            ),
            // Zed filling the screen: straight up, 12 below the menu bar.
            (rect(0.0, 33.0, 1470.0, 833.0), Edge::Top, (0.0, -788.0)),
            // GitHub Desktop, wide and tall: to the right edge, up.
            (
                rect(173.0, 33.0, 1297.0, 833.0),
                Edge::Right,
                (1458.0, -635.0),
            ),
            // Terminal, above centre: to the top.
            (rect(433.0, 105.0, 580.0, 385.0), Edge::Top, (406.0, -340.0)),
            // TextEdit, below centre: to the bottom, under the Dock.
            (
                rect(535.0, 600.0, 400.0, 300.0),
                Edge::Bottom,
                (535.0, 944.0),
            ),
        ];
        for (frame, edge, expected) in cases {
            let push = push_aside(MAC_AREA, frame, MAC_SLIVER).unwrap();
            assert_eq!(push.edge, edge, "{frame:?}");
            assert!(
                close(landed(frame, push), expected),
                "{frame:?} landed at {:?}, the Mac at {expected:?}",
                landed(frame, push)
            );
        }
    }

    #[test]
    fn exactly_the_sliver_stays_on_screen() {
        let frame = rect(100.0, 100.0, 500.0, 350.0);
        let push = push_aside(MAC_AREA, frame, 75.0).unwrap();
        let (x, _) = landed(frame, push);
        assert!((x + frame.width - 75.0).abs() < 1e-9);

        // A window narrower than the sliver stays whole at the edge.
        let narrow = rect(1300.0, 400.0, 40.0, 100.0);
        let push = push_aside(MAC_AREA, narrow, 75.0).unwrap();
        assert_eq!(push.edge, Edge::Right);
        assert!((landed(narrow, push).0 - (1470.0 - 40.0)).abs() < 1e-9);
    }

    #[test]
    fn dead_centre_goes_up_and_a_window_at_its_edge_stays() {
        let centred = rect(535.0, 344.5, 400.0, 300.0);
        let push = push_aside(MAC_AREA, centred, MAC_SLIVER).unwrap();
        assert_eq!(push.edge, Edge::Top);
        assert_eq!(push.dx, 0.0);

        let aside = rect(-588.0, 173.0, 600.0, 380.0);
        assert_eq!(push_aside(MAC_AREA, aside, MAC_SLIVER), None);
        assert_eq!(
            push_aside(MAC_AREA, rect(f64::NAN, 0.0, 10.0, 10.0), MAC_SLIVER),
            None
        );
    }

    // ----- snapshot rules

    fn output() -> Output {
        Output {
            id: OutputId::from("eDP-1"),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(LogicalOutput {
                position: LogicalPoint::default(),
                size: LogicalSize {
                    width: 1536.0,
                    height: 864.0,
                },
                scale: 1.25,
                transform: "Normal".into(),
            }),
        }
    }

    fn workspace(id: u64, name: Option<&str>, active: bool) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: name.map(str::to_owned),
            output: Some(OutputId::from("eDP-1")),
            urgent: false,
            active,
            focused: active,
            active_window: None,
        }
    }

    fn window(id: u64, workspace: u64, floating: bool, at: Option<(f64, f64)>) -> Window {
        Window {
            id: WindowId(id),
            title: None,
            app_id: Some("org.rmac.TextEditor".into()),
            pid: None,
            workspace: Some(WorkspaceId(workspace)),
            focused: false,
            floating,
            urgent: false,
            focus_timestamp: None,
            layout: WindowLayout {
                scrolling_position: None,
                tile_size: LogicalSize {
                    width: 600.0,
                    height: 400.0,
                },
                tile_position_in_view: at.map(|(x, y)| LogicalPoint { x, y }),
                ..WindowLayout::default()
            },
        }
    }

    fn snapshot(windows: Vec<Window>, focus: Option<u64>) -> Snapshot {
        Snapshot {
            outputs: vec![output()],
            workspaces: vec![
                workspace(1, Some("Desktop"), true),
                workspace(2, Some(PARKING_WORKSPACE), false),
                workspace(3, None, false),
            ],
            windows,
            focus: FocusState {
                window: focus.map(WindowId),
                ..FocusState::default()
            },
            ..Snapshot::default()
        }
    }

    fn moves(actions: &[Action]) -> Vec<(u64, f64, f64)> {
        actions
            .iter()
            .map(|action| match action {
                Action::MoveWindowBy { window, dx, dy } => (window.0, dx.0, dy.0),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn only_floating_windows_on_the_showing_space_move() {
        let snapshot = snapshot(
            vec![
                window(10, 1, true, Some((100.0, 100.0))),
                window(11, 1, false, Some((200.0, 100.0))),
                window(12, 2, true, Some((100.0, 100.0))),
                window(13, 3, true, Some((100.0, 100.0))),
                window(14, 1, true, None),
            ],
            Some(10),
        );
        let (revealed, actions) = reveal(&snapshot, 29.0, 75.0).unwrap();
        assert_eq!(revealed.moved.len(), 1);
        assert_eq!(revealed.moved[0].window, WindowId(10));
        assert_eq!(revealed.workspaces, vec![WorkspaceId(1)]);
        assert_eq!(revealed.focus, Some(WindowId(10)));
        let (id, dx, _) = moves(&actions)[0];
        assert_eq!(id, 10);
        // Up and to the left: the left edge, 75 px still showing.
        assert!((100.0 + dx + 600.0 - 75.0).abs() < 1e-9);

        let empty = snapshot_without_windows();
        assert_eq!(reveal(&empty, 29.0, 75.0), None);
    }

    fn snapshot_without_windows() -> Snapshot {
        snapshot(Vec::new(), None)
    }

    #[test]
    fn restore_returns_each_window_exactly_from_where_it_landed() {
        let before = snapshot(vec![window(10, 1, true, Some((487.2, 170.4)))], None);
        let (revealed, actions) = reveal(&before, 29.0, 75.0).unwrap();
        let (_, dx, dy) = moves(&actions)[0];

        // The compositor stopped the window a little short of the request.
        let after = snapshot(
            vec![window(10, 1, true, Some((487.2 + dx + 3.2, 170.4 + dy)))],
            None,
        );
        let back = moves(&restore(&after, &revealed));
        assert_eq!(back.len(), 1);
        assert!((487.2 + dx + 3.2 + back[0].1 - 487.2).abs() < 1e-9);
        assert!((170.4 + dy + back[0].2 - 170.4).abs() < 1e-9);

        // On a hidden Space the window has no position: undo the request.
        let hidden = snapshot(vec![window(10, 3, true, None)], None);
        let back = moves(&restore(&hidden, &revealed));
        assert_eq!(back, vec![(10, -dx, -dy)]);

        // A closed window is simply forgotten.
        assert!(restore(&snapshot_without_windows(), &revealed).is_empty());
    }

    #[test]
    fn focusing_a_window_or_leaving_the_space_brings_windows_back() {
        let at = Some((100.0, 100.0));
        let (mut revealed, _) = reveal(
            &snapshot(vec![window(10, 1, true, at)], Some(10)),
            29.0,
            75.0,
        )
        .unwrap();

        // The wallpaper took the keyboard: nothing to do.
        assert!(!revealed.should_restore(&snapshot(vec![window(10, 1, true, at)], None)));
        // A sliver was clicked: the window that was focused before.
        assert!(revealed.should_restore(&snapshot(vec![window(10, 1, true, at)], Some(10))));

        let (mut revealed, _) = reveal(
            &snapshot(vec![window(10, 1, true, at)], Some(10)),
            29.0,
            75.0,
        )
        .unwrap();
        // Unchanged focus is not a new activation.
        assert!(!revealed.should_restore(&snapshot(vec![window(10, 1, true, at)], Some(10))));
        // A newly opened item's window took focus.
        assert!(revealed.should_restore(&snapshot(
            vec![window(10, 1, true, at), window(20, 1, true, at)],
            Some(20)
        )));

        let (mut revealed, _) =
            reveal(&snapshot(vec![window(10, 1, true, at)], None), 29.0, 75.0).unwrap();
        let mut elsewhere = snapshot(vec![window(10, 1, true, at)], None);
        elsewhere.workspaces[0].active = false;
        elsewhere.workspaces[2].active = true;
        assert!(revealed.should_restore(&elsewhere));
    }

    #[test]
    fn recovery_pulls_back_only_windows_left_at_an_edge() {
        let snapshot = snapshot(
            vec![
                // Left at the left edge with 75.2 px showing.
                window(10, 1, true, Some((-524.8, 200.0))),
                // Left at the top edge.
                window(11, 1, true, Some((300.0, 29.0 + 75.0 - 400.0))),
                // An ordinary window, partly off the right edge by choice.
                window(12, 1, true, Some((1200.0, 200.0))),
                window(13, 1, true, Some((100.0, 100.0))),
            ],
            None,
        );
        let actions = moves(&recover(&snapshot, 29.0, 75.0));
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], (10, 524.8, 0.0));
        assert_eq!(actions[1].0, 11);
        assert!((actions[1].2 - (400.0 - 75.0)).abs() < 1e-9);
    }
}
