//! macOS's visible frame for windows the shell does not draw (third-party
//! apps): the screen below the menu bar and beside the Dock.
//!
//! macOS constrains every new window to its screen's visible frame
//! (`-[NSWindow constrainFrameRect:toScreen:]`): a window bigger than the
//! frame is shrunk to fit, one that sits outside it is moved in. It keeps a
//! dragged window's title bar below the menu bar but lets the rest of the
//! window go under the Dock or off screen. When the Dock or a display
//! changes, windows that now sit under the menu bar or the Dock are moved
//! back in.
//!
//! niri floats every Lulo OS window (shell.kdl `open-floating`) but only
//! keeps 75 px of a floating window on screen, so a browser opening at its
//! remembered 4000 × 4000 or at 0,0 sat under the top bar and the Dock.
//! [`Placer`] follows the compositor event stream and answers with the
//! floating moves and resizes that put such windows where a Mac would.
//! Lulo's own apps size and centre themselves before their first frame
//! (`rmac_ui::window::fit_to_display_after_first_frame`), so new Lulo
//! windows are left alone and never jump.
//!
//! Every rectangle here is output-local logical pixels, the space of
//! [`crate::WindowLayout::tile_position_in_view`].

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Action, Distance, Event, LogicalRect, Output, OutputId, State, Window, WindowId,
    PARKING_WORKSPACE,
};

/// Compositor rounding: frames snap to physical pixels.
const SLACK: f64 = 1.0;
/// How many layout reports of a new window [`Placer`] checks: a client may
/// commit its first buffer at one size and grow to a remembered one just
/// after. Our own move and resize report too.
const NEW_WINDOW_REPORTS: u8 = 4;
/// How many times a new window is fitted before the client's own size wins
/// (one that refuses to shrink below its minimum).
const NEW_WINDOW_FITS: u8 = 2;

/// The screen edge the Dock sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockEdge {
    Bottom,
    Left,
    Right,
}

/// Which outputs show the Dock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DockOutputs {
    All,
    /// The primary output: the one at the layout's origin.
    Primary,
    Named(OutputId),
}

/// What the shell reserves on each output, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Reservation {
    /// The menu bar, on every output.
    pub menu_bar: f64,
    pub dock_edge: DockEdge,
    /// The Dock's reservation: zero when it hides or reserves nothing.
    pub dock: f64,
    pub dock_outputs: DockOutputs,
}

impl Default for Reservation {
    fn default() -> Self {
        Self {
            menu_bar: 29.0,
            dock_edge: DockEdge::Bottom,
            dock: 89.0,
            dock_outputs: DockOutputs::All,
        }
    }
}

/// Reserved edges of one output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Reservation {
    /// The edges reserved on `output`, given every output (to find the
    /// primary one).
    pub fn insets<'a>(
        &self,
        output: &OutputId,
        outputs: impl IntoIterator<Item = &'a Output>,
    ) -> Insets {
        let mut insets = Insets {
            top: self.menu_bar.max(0.0),
            ..Insets::default()
        };
        let shows_dock = match &self.dock_outputs {
            DockOutputs::All => true,
            DockOutputs::Named(name) => name == output,
            DockOutputs::Primary => primary_output(outputs).as_ref() == Some(output),
        };
        if shows_dock && self.dock > 0.0 {
            match self.dock_edge {
                DockEdge::Bottom => insets.bottom = self.dock,
                DockEdge::Left => insets.left = self.dock,
                DockEdge::Right => insets.right = self.dock,
            }
        }
        insets
    }
}

/// The output at the layout's origin, or the first enabled one.
fn primary_output<'a>(outputs: impl IntoIterator<Item = &'a Output>) -> Option<OutputId> {
    let enabled: Vec<&Output> = outputs.into_iter().filter(|o| o.enabled()).collect();
    enabled
        .iter()
        .find(|output| {
            output.logical.as_ref().is_some_and(|logical| {
                logical.position.x.abs() < SLACK && logical.position.y.abs() < SLACK
            })
        })
        .or_else(|| enabled.first())
        .map(|output| output.id.clone())
}

/// `output`'s visible frame, output-local. `None` when it has no logical
/// geometry or nothing is left between the reserved edges.
pub fn visible_frame(output: &Output, insets: Insets) -> Option<LogicalRect> {
    let logical = output.logical.as_ref()?;
    if !logical.size.is_valid() {
        return None;
    }
    let width = logical.size.width - insets.left - insets.right;
    let height = logical.size.height - insets.top - insets.bottom;
    (width > 0.0 && height > 0.0).then_some(LogicalRect {
        x: insets.left,
        y: insets.top,
        width,
        height,
    })
}

/// Whether `frame` lies inside `area`, give or take a pixel.
pub fn inside(frame: LogicalRect, area: LogicalRect) -> bool {
    frame.x >= area.x - SLACK
        && frame.y >= area.y - SLACK
        && frame.x + frame.width <= area.x + area.width + SLACK
        && frame.y + frame.height <= area.y + area.height + SLACK
}

/// A new window's frame as macOS constrains it: no bigger than `visible`,
/// then moved the least distance that puts it inside, the top-left edges
/// winning. `None` when it is already inside.
pub fn constrain_new(frame: LogicalRect, visible: LogicalRect) -> Option<LogicalRect> {
    if inside(frame, visible) {
        return None;
    }
    let width = frame.width.min(visible.width);
    let height = frame.height.min(visible.height);
    let x = frame
        .x
        .min(visible.x + visible.width - width)
        .max(visible.x);
    let y = frame
        .y
        .min(visible.y + visible.height - height)
        .max(visible.y);
    Some(LogicalRect {
        x,
        y,
        width,
        height,
    })
}

/// A dragged window keeps its title bar below the menu bar; the rest may go
/// anywhere niri allows. `None` when its top edge is already low enough.
pub fn constrain_title_bar(frame: LogicalRect, visible: LogicalRect) -> Option<LogicalRect> {
    (frame.y < visible.y - SLACK).then_some(LogicalRect {
        y: visible.y,
        ..frame
    })
}

/// Lulo's own windows, which place themselves.
pub fn is_lulo_window(app_id: Option<&str>) -> bool {
    app_id.is_some_and(|id| id.starts_with("org.rmac.") || id.starts_with("dev.rmac."))
}

/// The actions that take floating `window` from `from` to `to`: a resize
/// (which keeps the top-left corner) and a relative move.
pub fn frame_actions(window: WindowId, from: LogicalRect, to: LogicalRect) -> Vec<Action> {
    let mut actions = Vec::new();
    if (to.width - from.width).abs() >= 0.5 || (to.height - from.height).abs() >= 0.5 {
        actions.push(Action::ResizeWindow {
            window,
            width: Distance(to.width),
            height: Distance(to.height),
        });
    }
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if dx.abs() >= 0.5 || dy.abs() >= 0.5 {
        actions.push(Action::MoveWindowBy {
            window,
            dx: Distance(dx),
            dy: Distance(dy),
        });
    }
    actions
}

#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    reports: u8,
    fits: u8,
    /// A new Lulo window: it centres itself just after mapping, so it is
    /// not touched at all until it has settled.
    lulo: bool,
}

/// Follows the compositor and keeps windows in the visible frame.
#[derive(Debug, Default)]
pub struct Placer {
    reservation: Reservation,
    known: BTreeSet<WindowId>,
    pending: BTreeMap<WindowId, Pending>,
    frames: BTreeMap<OutputId, LogicalRect>,
}

impl Placer {
    pub fn new(reservation: Reservation) -> Self {
        Self {
            reservation,
            ..Self::default()
        }
    }

    /// React to `event`, already applied to `state`. `suspended` is true
    /// while the shell has windows pushed aside on purpose (click wallpaper
    /// to reveal desktop): only new windows are placed then.
    pub fn on_event(&mut self, event: &Event, state: &State, suspended: bool) -> Vec<Action> {
        match event {
            Event::Snapshot { .. } | Event::WindowsReplaced { .. } => {
                // Windows already open stay where they are.
                self.known = state.windows.keys().copied().collect();
                self.pending.clear();
                self.frames = self.visible_frames(state);
                Vec::new()
            }
            Event::WindowRemoved { id } => {
                self.known.remove(id);
                self.pending.remove(id);
                Vec::new()
            }
            Event::WindowUpserted { window } => {
                if self.known.insert(window.id) {
                    self.pending.insert(
                        window.id,
                        Pending {
                            lulo: is_lulo_window(window.app_id.as_deref()),
                            ..Pending::default()
                        },
                    );
                }
                self.place(window.id, state, suspended)
            }
            Event::OutputsReplaced { .. } => {
                let frames = self.visible_frames(state);
                let changed: Vec<(OutputId, Option<LogicalRect>)> = frames
                    .iter()
                    .filter(|(output, frame)| self.frames.get(*output) != Some(*frame))
                    .map(|(output, _)| (output.clone(), self.frames.get(output).copied()))
                    .collect();
                self.frames = frames;
                if suspended {
                    return Vec::new();
                }
                changed
                    .into_iter()
                    .flat_map(|(output, old)| self.refit_output(&output, old, state))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// New Dock or menu-bar geometry (Desktop & Dock changed). Windows that
    /// now sit under the bar or the Dock move back in.
    pub fn set_reservation(
        &mut self,
        reservation: Reservation,
        state: &State,
        suspended: bool,
    ) -> Vec<Action> {
        if reservation == self.reservation {
            return Vec::new();
        }
        self.reservation = reservation;
        let old = std::mem::take(&mut self.frames);
        self.frames = self.visible_frames(state);
        if suspended {
            return Vec::new();
        }
        let outputs: Vec<OutputId> = self.frames.keys().cloned().collect();
        outputs
            .into_iter()
            .flat_map(|output| {
                let before = old.get(&output).copied();
                self.refit_output(&output, before, state)
            })
            .collect()
    }

    fn visible_frames(&self, state: &State) -> BTreeMap<OutputId, LogicalRect> {
        state
            .outputs
            .values()
            .filter_map(|output| {
                let insets = self.reservation.insets(&output.id, state.outputs.values());
                Some((output.id.clone(), visible_frame(output, insets)?))
            })
            .collect()
    }

    /// Where `window` is and the visible frame of its output, for a
    /// floating window on a real Space.
    fn placement(&self, window: &Window, state: &State) -> Option<(LogicalRect, LogicalRect)> {
        if !window.floating {
            // Full screen, maximised or tiled: the compositor sizes it.
            return None;
        }
        let workspace = state.workspaces.get(&window.workspace?)?;
        if workspace.name.as_deref() == Some(PARKING_WORKSPACE) {
            return None;
        }
        let visible = *self.frames.get(workspace.output.as_ref()?)?;
        let position = window.layout.tile_position_in_view?;
        let size = window.layout.tile_size;
        if !(size.is_valid() && size.width > 0.0 && size.height > 0.0) {
            return None;
        }
        Some((
            LogicalRect {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            },
            visible,
        ))
    }

    fn place(&mut self, id: WindowId, state: &State, suspended: bool) -> Vec<Action> {
        let Some(window) = state.windows.get(&id) else {
            return Vec::new();
        };
        let Some((frame, visible)) = self.placement(window, state) else {
            return Vec::new();
        };
        if let Some(pending) = self.pending.get_mut(&id) {
            pending.reports += 1;
            if pending.lulo {
                if pending.reports >= NEW_WINDOW_REPORTS {
                    self.pending.remove(&id);
                }
                return Vec::new();
            }
            let mut actions = Vec::new();
            let target = constrain_new(frame, visible);
            if let Some(target) = target.filter(|_| pending.fits < NEW_WINDOW_FITS) {
                pending.fits += 1;
                actions = frame_actions(id, frame, target);
            }
            // Fitted and now inside: placed. From here on it is the user's.
            let placed = pending.fits > 0 && target.is_none();
            if placed || pending.reports >= NEW_WINDOW_REPORTS {
                self.pending.remove(&id);
            }
            return actions;
        }
        if suspended {
            return Vec::new();
        }
        constrain_title_bar(frame, visible)
            .map(|target| frame_actions(id, frame, target))
            .unwrap_or_default()
    }

    /// Re-fit the windows of `output` that were inside its `old` visible
    /// frame (or every window, when there was none) and are not inside the
    /// new one. A window the user dragged partly under the Dock stays.
    fn refit_output(
        &self,
        output: &OutputId,
        old: Option<LogicalRect>,
        state: &State,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        for window in state.windows.values() {
            let on_output = window
                .workspace
                .and_then(|id| state.workspaces.get(&id))
                .and_then(|workspace| workspace.output.as_ref())
                == Some(output);
            if !on_output {
                continue;
            }
            let Some((frame, visible)) = self.placement(window, state) else {
                continue;
            };
            if old.is_some_and(|old| !inside(frame, old)) {
                continue;
            }
            if let Some(target) = constrain_new(frame, visible) {
                actions.extend(frame_actions(window.id, frame, target));
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LogicalOutput, LogicalPoint, LogicalSize, OutputMode, PhysicalSize, WindowLayout,
        Workspace, WorkspaceId,
    };

    fn output(name: &str, x: f64, width: f64, height: f64) -> Output {
        Output {
            id: OutputId::from(name),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: vec![OutputMode {
                physical_size: PhysicalSize {
                    width: width as u32,
                    height: height as u32,
                },
                refresh_millihz: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(LogicalOutput {
                position: LogicalPoint { x, y: 0.0 },
                size: LogicalSize { width, height },
                scale: 1.0,
                transform: "Normal".into(),
            }),
        }
    }

    fn window(id: u64, app_id: &str, frame: (f64, f64, f64, f64)) -> Window {
        Window {
            always_on_top: false,
            id: WindowId(id),
            title: None,
            app_id: Some(app_id.into()),
            pid: None,
            workspace: Some(WorkspaceId(1)),
            focused: false,
            floating: true,
            urgent: false,
            focus_timestamp: None,
            layout: WindowLayout {
                tile_size: LogicalSize {
                    width: frame.2,
                    height: frame.3,
                },
                tile_position_in_view: Some(LogicalPoint {
                    x: frame.0,
                    y: frame.1,
                }),
                ..WindowLayout::default()
            },
        }
    }

    fn state(windows: Vec<Window>) -> State {
        let mut state = State::default();
        state.apply(Event::OutputsReplaced {
            outputs: vec![output("eDP-1", 0.0, 1536.0, 864.0)],
        });
        state.apply(Event::WorkspacesReplaced {
            workspaces: vec![Workspace {
                id: WorkspaceId(1),
                index: 1,
                name: None,
                output: Some(OutputId::from("eDP-1")),
                urgent: false,
                active: true,
                focused: true,
                active_window: None,
            }],
        });
        state.apply(Event::WindowsReplaced { windows });
        state
    }

    fn open(placer: &mut Placer, state: &mut State, window: Window) -> Vec<Action> {
        let event = Event::WindowUpserted { window };
        state.apply(event.clone());
        placer.on_event(&event, state, false)
    }

    fn started() -> (Placer, State) {
        let state = state(Vec::new());
        let mut placer = Placer::new(Reservation::default());
        let snapshot = Event::Snapshot {
            snapshot: state.snapshot(),
        };
        assert!(placer.on_event(&snapshot, &state, false).is_empty());
        (placer, state)
    }

    #[test]
    fn a_huge_new_window_at_the_origin_is_shrunk_into_the_visible_frame() {
        let (mut placer, mut state) = started();
        let actions = open(
            &mut placer,
            &mut state,
            window(5, "firefox", (0.0, 0.0, 4000.0, 4000.0)),
        );
        // 1536 × 864 less the 29 pt bar and the 89 pt Dock: 1536 × 746 at 0,29.
        assert_eq!(
            actions,
            vec![
                Action::ResizeWindow {
                    window: WindowId(5),
                    width: Distance(1536.0),
                    height: Distance(746.0),
                },
                Action::MoveWindowBy {
                    window: WindowId(5),
                    dx: Distance(0.0),
                    dy: Distance(29.0),
                },
            ]
        );
    }

    #[test]
    fn a_window_that_fits_but_sits_under_the_dock_only_moves_up() {
        let (mut placer, mut state) = started();
        let actions = open(
            &mut placer,
            &mut state,
            window(6, "org.gnome.TextEditor", (300.0, 400.0, 800.0, 420.0)),
        );
        // Bottom 820 > 775: moved up 45 so it ends at the Dock.
        assert_eq!(
            actions,
            vec![Action::MoveWindowBy {
                window: WindowId(6),
                dx: Distance(0.0),
                dy: Distance(-45.0),
            }]
        );
    }

    #[test]
    fn lulo_windows_and_windows_already_inside_are_left_alone() {
        let (mut placer, mut state) = started();
        assert!(open(
            &mut placer,
            &mut state,
            window(7, "org.rmac.Files", (0.0, 0.0, 4000.0, 4000.0)),
        )
        .is_empty());
        assert!(open(
            &mut placer,
            &mut state,
            window(8, "foot", (100.0, 100.0, 600.0, 400.0)),
        )
        .is_empty());
    }

    #[test]
    fn full_screen_and_parked_windows_are_not_placed() {
        let (mut placer, mut state) = started();
        let mut full_screen = window(9, "mpv", (0.0, 0.0, 1536.0, 864.0));
        full_screen.floating = false;
        assert!(open(&mut placer, &mut state, full_screen).is_empty());
    }

    #[test]
    fn a_window_that_refuses_to_shrink_is_fitted_twice_then_left() {
        let (mut placer, mut state) = started();
        let big = window(10, "stubborn", (0.0, 0.0, 4000.0, 4000.0));
        assert!(!open(&mut placer, &mut state, big.clone()).is_empty());
        assert!(!open(&mut placer, &mut state, big.clone()).is_empty());
        assert!(open(&mut placer, &mut state, big.clone()).is_empty());
    }

    #[test]
    fn a_dragged_window_keeps_its_title_bar_below_the_menu_bar_only() {
        let (mut placer, mut state) = started();
        assert!(open(
            &mut placer,
            &mut state,
            window(11, "foot", (100.0, 100.0, 600.0, 400.0)),
        )
        .is_empty());
        for _ in 0..NEW_WINDOW_REPORTS {
            open(
                &mut placer,
                &mut state,
                window(11, "foot", (100.0, 100.0, 600.0, 400.0)),
            );
        }
        // Dropped half under the Dock: allowed, as on the Mac.
        assert!(open(
            &mut placer,
            &mut state,
            window(11, "foot", (100.0, 600.0, 600.0, 400.0)),
        )
        .is_empty());
        // Dropped with its title bar under the menu bar: moved down.
        assert_eq!(
            open(
                &mut placer,
                &mut state,
                window(11, "foot", (100.0, 5.0, 600.0, 400.0)),
            ),
            vec![Action::MoveWindowBy {
                window: WindowId(11),
                dx: Distance(0.0),
                dy: Distance(24.0),
            }]
        );
        // Not while the desktop is revealed.
        let event = Event::WindowUpserted {
            window: window(11, "foot", (100.0, -300.0, 600.0, 400.0)),
        };
        state.apply(event.clone());
        assert!(placer.on_event(&event, &state, true).is_empty());
    }

    #[test]
    fn a_bigger_dock_moves_the_windows_now_under_it() {
        let (mut placer, mut state) = started();
        for (id, frame) in [
            // Fully inside, its bottom at 760: under a 128 pt Dock.
            (12, (100.0, 100.0, 600.0, 660.0)),
            // Already dragged partly under the old Dock: stays.
            (13, (700.0, 600.0, 600.0, 400.0)),
            // High up: unaffected.
            (14, (100.0, 40.0, 300.0, 200.0)),
        ] {
            let actions = open(&mut placer, &mut state, window(id, "foot", frame));
            assert!(actions.is_empty() || id == 13);
        }
        let actions = placer.set_reservation(
            Reservation {
                dock: 128.0,
                ..Reservation::default()
            },
            &state,
            false,
        );
        assert_eq!(
            actions,
            vec![Action::MoveWindowBy {
                window: WindowId(12),
                dx: Distance(0.0),
                dy: Distance(-24.0),
            }]
        );
        // Turning on hiding gives the space back and moves nothing.
        assert!(placer
            .set_reservation(
                Reservation {
                    dock: 0.0,
                    ..Reservation::default()
                },
                &state,
                false,
            )
            .is_empty());
    }

    #[test]
    fn a_smaller_display_refits_the_windows_that_were_inside() {
        let (mut placer, mut state) = started();
        open(
            &mut placer,
            &mut state,
            window(15, "foot", (400.0, 100.0, 1000.0, 600.0)),
        );
        let event = Event::OutputsReplaced {
            outputs: vec![output("eDP-1", 0.0, 1280.0, 720.0)],
        };
        state.apply(event.clone());
        // 1280 × 720: frame 0,29 1280 × 602, so it moves left and up.
        assert_eq!(
            placer.on_event(&event, &state, false),
            vec![Action::MoveWindowBy {
                window: WindowId(15),
                dx: Distance(-120.0),
                dy: Distance(-69.0),
            }]
        );
    }

    #[test]
    fn the_dock_reserves_its_own_edge_on_its_own_outputs() {
        let reservation = Reservation {
            dock_edge: DockEdge::Left,
            dock_outputs: DockOutputs::Primary,
            ..Reservation::default()
        };
        let outputs = [
            output("HDMI-1", 1536.0, 1920.0, 1080.0),
            output("eDP-1", 0.0, 1536.0, 864.0),
        ];
        assert_eq!(
            reservation.insets(&OutputId::from("eDP-1"), &outputs),
            Insets {
                top: 29.0,
                left: 89.0,
                ..Insets::default()
            }
        );
        assert_eq!(
            reservation.insets(&OutputId::from("HDMI-1"), &outputs),
            Insets {
                top: 29.0,
                ..Insets::default()
            }
        );
    }
}
