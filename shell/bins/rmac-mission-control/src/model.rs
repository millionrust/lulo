//! Mission Control, App Exposé, Spaces and hot-corner model
//! (docs/decisions/0014-mission-control.md).
//!
//! Every size here was measured on the owner's Mac (macOS 26.2, 1470 × 956 pt
//! screen) and is mirrored in design-lab/mission-control.html. Nothing here
//! touches GPUI, sockets or niri, so the behaviour is tested on every
//! platform.

use rmac_compositor::{
    window_is_parked, Action, OutputId, Snapshot, Timestamp, Window, WindowId, Workspace,
    WorkspaceId, PARKING_WORKSPACE,
};
use rmac_shell_settings::{HotCornerAction, HotCornerSettings};

// ---------------------------------------------------------------------------
// Commands

/// One word from the niri binds, the Dock or a hot corner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    /// ⌃↑: Mission Control, or close it when it is open.
    MissionControl,
    /// ⌃↓: App Exposé for the focused application.
    AppWindows,
    /// F11: show the desktop, or bring the windows back.
    ShowDesktop,
    /// ⌃→ / ⌃←: the next or previous Space on the focused display.
    NextSpace,
    PreviousSpace,
    Cancel,
}

impl Command {
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "mission-control" => Self::MissionControl,
            "app-windows" => Self::AppWindows,
            "show-desktop" => Self::ShowDesktop,
            "next-space" => Self::NextSpace,
            "previous-space" => Self::PreviousSpace,
            "cancel" => Self::Cancel,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissionControl => "mission-control",
            Self::AppWindows => "app-windows",
            Self::ShowDesktop => "show-desktop",
            Self::NextSpace => "next-space",
            Self::PreviousSpace => "previous-space",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    MissionControl,
    AppWindows,
}

// ---------------------------------------------------------------------------
// Measured geometry (logical points)

/// The Spaces bar, collapsed and while the pointer is over it.
pub const BAR_COLLAPSED: f32 = 72.0;
pub const BAR_EXPANDED: f32 = 164.0;
/// The collapsed "Desktop" pill: 24 tall, top 40, 10 side padding, 13 pt.
pub const PILL_TOP: f32 = 40.0;
pub const PILL_HEIGHT: f32 = 24.0;
pub const PILL_PADDING: f32 = 10.0;
pub const PILL_LABEL: f32 = 13.0;
/// Expanded Space thumbnails: 138 wide at the screen's aspect, top 46, on a
/// 170 pitch, with an 11 pt label whose line box starts at 142.
pub const THUMB_TOP: f32 = 46.0;
pub const THUMB_WIDTH: f32 = 138.0;
pub const THUMB_PITCH: f32 = 170.0;
pub const THUMB_RADIUS: f32 = 2.0;
pub const THUMB_LABEL: f32 = 11.0;
pub const THUMB_LABEL_TOP: f32 = 142.0;
pub const THUMB_LABEL_LINE: f32 = 13.0;
/// The current Space's ring: 3 pt, 1 pt outside the thumbnail, radius 6.
pub const THUMB_RING: f32 = 3.0;
pub const THUMB_RING_GAP: f32 = 1.0;
pub const THUMB_RING_RADIUS: f32 = 6.0;
/// The remove button centred on a hovered thumbnail's top-left corner.
pub const REMOVE_DIAMETER: f32 = 22.0;
pub const REMOVE_GLYPH: f32 = 8.0;
/// The + button: 32 circle, centre 34 from the right edge, y 44 or 90.
pub const ADD_DIAMETER: f32 = 32.0;
pub const ADD_CENTRE_FROM_RIGHT: f32 = 34.0;
pub const ADD_CENTRE_COLLAPSED: f32 = 44.0;
pub const ADD_CENTRE_EXPANDED: f32 = 90.0;
pub const PLUS_SIZE: f32 = 19.0;
pub const PLUS_STROKE: f32 = 2.0;
/// Window areas: Mission Control spans x 20 … W − 20 and y bar + 40 …
/// H − 104; App Exposé spans y 52 … H − 100.
pub const AREA_SIDE: f32 = 20.0;
pub const AREA_BELOW_BAR: f32 = 40.0;
pub const AREA_ABOVE_DOCK: f32 = 104.0;
pub const EXPOSE_TOP: f32 = 52.0;
pub const EXPOSE_BOTTOM: f32 = 100.0;
/// The hover ring: 5 pt, 1 pt outside the window.
pub const HOVER_RING: f32 = 5.0;
pub const HOVER_GAP: f32 = 1.0;
/// Mission Control's hover title capsule.
pub const TITLE_HEIGHT: f32 = 22.5;
pub const TITLE_SIZE: f32 = 18.0;
pub const TITLE_PADDING: f32 = 14.5;
/// App Exposé's captions: 13 pt, 9 below the window, in a 32 pt band.
pub const CAPTION_SIZE: f32 = 13.0;
pub const CAPTION_LINE: f32 = 16.0;
pub const CAPTION_GAP: f32 = 9.0;
pub const CAPTION_BAND: f32 = 32.0;
/// Smallest spacing kept between windows (S: the tightest gaps in the
/// captures were ≈ 44 across and ≈ 15 down).
pub const GAP_X: f32 = 40.0;
pub const GAP_Y: f32 = 24.0;
/// The Mission Control motion token (shell.kdl overview-open-close).
pub const ANIMATION_MS: u64 = 350;
/// A later window overlapping by more than this hides part of a window, so
/// its screen pixels are not its own.
const OCCLUSION_TOLERANCE: f32 = 2.0;

/// Named workspaces that rmac created for Mission Control's +.
pub const SPACE_NAME_PREFIX: &str = "rmac-space-";

// ---------------------------------------------------------------------------
// Rectangles

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The part of `self` inside a `width` × `height` output.
    pub fn clipped(&self, width: f32, height: f32) -> Self {
        let left = self.x.max(0.0);
        let top = self.y.max(0.0);
        let right = self.right().min(width);
        let bottom = self.bottom().min(height);
        Self::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    pub fn is_empty(&self) -> bool {
        self.width < 1.0 || self.height < 1.0
    }

    /// Whether the two overlap by more than `tolerance` on both axes.
    pub fn overlaps(&self, other: &Self, tolerance: f32) -> bool {
        let across = self.right().min(other.right()) - self.x.max(other.x);
        let down = self.bottom().min(other.bottom()) - self.y.max(other.y);
        across > tolerance && down > tolerance
    }

    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self::new(
            mix(self.x, to.x),
            mix(self.y, to.y),
            mix(self.width, to.width),
            mix(self.height, to.height),
        )
    }
}

/// The Mission Control curve (cubic-bezier .65 0 .35 1, as niri's
/// overview-open-close in shell.kdl), solved for `t` in 0…1.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (x1, y1, x2, y2) = (0.65_f32, 0.0_f32, 0.35_f32, 1.0_f32);
    let bezier = |a: f32, b: f32, s: f32| {
        let inv = 1.0 - s;
        3.0 * inv * inv * s * a + 3.0 * inv * s * s * b + s * s * s
    };
    // Bisection on x(s) = t; x is monotonic for these control points.
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        if bezier(x1, x2, mid) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    bezier(y1, y2, (low + high) / 2.0)
}

// ---------------------------------------------------------------------------
// Layout

/// Mission Control's window area for an output of `width` × `height` with
/// the Spaces bar `bar` tall.
pub fn mission_control_area(width: f32, height: f32, bar: f32) -> Rect {
    let top = bar + AREA_BELOW_BAR;
    Rect::new(
        AREA_SIDE,
        top,
        (width - 2.0 * AREA_SIDE).max(1.0),
        (height - AREA_ABOVE_DOCK - top).max(1.0),
    )
}

/// App Exposé's window area.
pub fn app_windows_area(width: f32, height: f32) -> Rect {
    Rect::new(
        AREA_SIDE,
        EXPOSE_TOP,
        (width - 2.0 * AREA_SIDE).max(1.0),
        (height - EXPOSE_BOTTOM - EXPOSE_TOP).max(1.0),
    )
}

/// Spread `frames` over `area` without overlap, keeping their on-screen order.
///
/// Windows are put in rows in top-to-bottom order (left to right inside a
/// row), every window takes the same scale (never above 1), and the space
/// left over is shared evenly around every window and every row. `band` is
/// reserved under each row for captions. Every row count is tried and the
/// largest scale wins. Fed App Exposé's three 557 × 370 terminals this gives
/// the Mac's measured (99, 52), (814, 52) and (456, 454).
pub fn layout(frames: &[Rect], area: Rect, gap_x: f32, gap_y: f32, band: f32) -> Vec<Rect> {
    let count = frames.len();
    if count == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| {
        let (ax, ay) = frames[a].centre();
        let (bx, by) = frames[b].centre();
        ay.total_cmp(&by).then(ax.total_cmp(&bx))
    });

    let mut best: Option<(f32, Vec<Vec<usize>>)> = None;
    for rows in 1..=count {
        let per_row = count.div_ceil(rows);
        let groups: Vec<Vec<usize>> = order
            .chunks(per_row)
            .map(|chunk| {
                let mut row = chunk.to_vec();
                row.sort_by(|&a, &b| frames[a].x.total_cmp(&frames[b].x));
                row
            })
            .collect();
        let mut scale = 1.0_f32;
        let mut tallest = 0.0_f32;
        for row in &groups {
            let width: f32 = row.iter().map(|&i| frames[i].width.max(1.0)).sum();
            scale = scale.min((area.width - gap_x * row.len() as f32) / width);
            tallest += row
                .iter()
                .map(|&i| frames[i].height.max(1.0))
                .fold(0.0, f32::max);
        }
        scale = scale.min((area.height - groups.len() as f32 * (gap_y + band)) / tallest);
        if best
            .as_ref()
            .is_none_or(|(best_scale, _)| scale > *best_scale + 1e-4)
        {
            best = Some((scale, groups));
        }
    }
    let (scale, groups) = best.expect("at least one row count was tried");
    let scale = scale.max(0.05);

    let heights: Vec<f32> = groups
        .iter()
        .map(|row| {
            row.iter()
                .map(|&i| frames[i].height.max(1.0) * scale)
                .fold(0.0, f32::max)
                + band
        })
        .collect();
    let spare_y = (area.height - heights.iter().sum::<f32>()) / groups.len() as f32;
    let mut placed = vec![Rect::default(); count];
    let mut y = area.y + spare_y / 2.0;
    for (row, height) in groups.iter().zip(&heights) {
        let used: f32 = row.iter().map(|&i| frames[i].width.max(1.0) * scale).sum();
        let spare_x = (area.width - used) / row.len() as f32;
        let mut x = area.x + spare_x / 2.0;
        for &i in row {
            let width = frames[i].width.max(1.0) * scale;
            let window_height = frames[i].height.max(1.0) * scale;
            placed[i] = Rect::new(
                x,
                y + (height - band - window_height) / 2.0,
                width,
                window_height,
            );
            x += width + spare_x;
        }
        y += height + spare_y;
    }
    placed
}

/// Centres of `count` Space thumbnails (and of the collapsed pills, which
/// grow into them) on a `width` wide output.
pub fn space_centres(count: usize, width: f32) -> Vec<f32> {
    let first = width / 2.0 - (count.saturating_sub(1)) as f32 * THUMB_PITCH / 2.0;
    (0..count).map(|i| first + i as f32 * THUMB_PITCH).collect()
}

// ---------------------------------------------------------------------------
// Scene

/// One window as Mission Control shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneWindow {
    pub id: WindowId,
    pub app_id: Option<String>,
    pub title: Option<String>,
    /// Where it sits now, output-local.
    pub frame: Rect,
    /// A window above it covers part of it, so a screen capture cannot show
    /// it; it is drawn as an icon card instead.
    pub occluded: bool,
}

/// The focused output and the windows Mission Control lays out on it,
/// bottom of the stack first.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub output: OutputId,
    pub origin: (f64, f64),
    pub width: f32,
    pub height: f32,
    pub workspace: WorkspaceId,
    pub windows: Vec<SceneWindow>,
}

fn stamp(timestamp: Option<Timestamp>) -> (u64, u32) {
    timestamp.map_or((0, 0), |time| (time.seconds, time.nanoseconds))
}

/// Shell surfaces that are not application windows.
pub fn is_application(window: &Window) -> bool {
    let Some(app_id) = window.app_id.as_deref() else {
        return true;
    };
    !app_id.starts_with("dev.rmac.")
        && !matches!(
            app_id,
            "org.rmac.Launcher" | "org.rmac.QuickSettings" | "org.rmac.NotificationCenter"
        )
}

/// The output that has keyboard focus.
pub fn focused_output(snapshot: &Snapshot) -> Option<OutputId> {
    snapshot
        .focus
        .output
        .clone()
        .or_else(|| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.focused)
                .and_then(|workspace| workspace.output.clone())
        })
        .or_else(|| {
            snapshot
                .outputs
                .iter()
                .find(|output| output.enabled())
                .map(|output| output.id.clone())
        })
}

fn active_workspace<'a>(snapshot: &'a Snapshot, output: &OutputId) -> Option<&'a Workspace> {
    snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.active && workspace.output.as_ref() == Some(output))
}

/// What Mission Control (every window of the current Space) or App Exposé
/// (the focused application's windows) shows on the focused output.
pub fn scene(snapshot: &Snapshot, mode: Mode) -> Option<Scene> {
    let output_id = focused_output(snapshot)?;
    let output = snapshot
        .outputs
        .iter()
        .find(|output| output.id == output_id)?;
    let logical = output.logical.as_ref()?;
    let width = logical.size.width as f32;
    let height = logical.size.height as f32;
    let workspace = active_workspace(snapshot, &output_id)?;
    let focused_app = snapshot
        .focus
        .window
        .and_then(|id| snapshot.windows.iter().find(|window| window.id == id))
        .and_then(|window| window.app_id.clone());
    if mode == Mode::AppWindows && focused_app.is_none() {
        return None;
    }

    let mut windows: Vec<&Window> = snapshot
        .windows
        .iter()
        .filter(|window| window.workspace == Some(workspace.id))
        .filter(|window| !window_is_parked(snapshot, window))
        .filter(|window| is_application(window))
        .collect();
    // niri raises a floating window when it is focused and keeps floating
    // windows over tiled ones, so this is the stacking order, bottom first.
    windows.sort_by_key(|window| (window.floating, stamp(window.focus_timestamp)));

    let frames: Vec<(usize, Rect)> = windows
        .iter()
        .enumerate()
        .filter_map(|(index, window)| {
            let position = window.layout.tile_position_in_view?;
            let size = window.layout.tile_size;
            let frame = Rect::new(
                position.x as f32,
                position.y as f32,
                size.width as f32,
                size.height as f32,
            )
            .clipped(width, height);
            (!frame.is_empty()).then_some((index, frame))
        })
        .collect();

    let scene_windows = frames
        .iter()
        .enumerate()
        .filter(|(_, (index, _))| {
            mode == Mode::MissionControl || windows[*index].app_id == focused_app
        })
        .map(|(position, (index, frame))| {
            let window = windows[*index];
            // Every window of the Space counts for occlusion, even in App
            // Exposé where only one application is shown.
            let occluded = frames[position + 1..]
                .iter()
                .any(|(_, above)| frame.overlaps(above, OCCLUSION_TOLERANCE));
            SceneWindow {
                id: window.id,
                app_id: window.app_id.clone(),
                title: window.title.clone(),
                frame: *frame,
                occluded,
            }
        })
        .collect();

    Some(Scene {
        output: output_id,
        origin: (logical.position.x, logical.position.y),
        width,
        height,
        workspace: workspace.id,
        windows: scene_windows,
    })
}

// ---------------------------------------------------------------------------
// Spaces

/// One Space in the bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Space {
    pub workspace: WorkspaceId,
    /// "Desktop" alone, otherwise "Desktop 1", "Desktop 2" …
    pub label: String,
    pub active: bool,
    pub named: bool,
    pub windows: Vec<WindowId>,
}

fn workspaces_on<'a>(snapshot: &'a Snapshot, output: &OutputId) -> Vec<&'a Workspace> {
    let mut workspaces: Vec<&Workspace> = snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.output.as_ref() == Some(output))
        .filter(|workspace| workspace.name.as_deref() != Some(PARKING_WORKSPACE))
        .collect();
    workspaces.sort_by_key(|workspace| workspace.index);
    workspaces
}

fn windows_on(snapshot: &Snapshot, workspace: WorkspaceId) -> Vec<WindowId> {
    snapshot
        .windows
        .iter()
        .filter(|window| window.workspace == Some(workspace))
        .filter(|window| is_application(window))
        .map(|window| window.id)
        .collect()
}

/// The Spaces on `output`, in niri's order. niri keeps an unnamed empty
/// workspace ready for new windows; that one is not a Space unless the user
/// is on it. The parking workspace never is.
pub fn spaces(snapshot: &Snapshot, output: &OutputId) -> Vec<Space> {
    let mut spaces: Vec<Space> = workspaces_on(snapshot, output)
        .into_iter()
        .filter_map(|workspace| {
            let windows = windows_on(snapshot, workspace.id);
            let named = workspace.name.is_some();
            (named || workspace.active || !windows.is_empty()).then(|| Space {
                workspace: workspace.id,
                label: String::new(),
                active: workspace.active,
                named,
                windows,
            })
        })
        .collect();
    let count = spaces.len();
    for (index, space) in spaces.iter_mut().enumerate() {
        space.label = if count == 1 {
            "Desktop".to_owned()
        } else {
            format!("Desktop {}", index + 1)
        };
    }
    spaces
}

/// niri's spare workspace on `output`: the last unnamed, empty one.
pub fn spare_workspace<'a>(snapshot: &'a Snapshot, output: &OutputId) -> Option<&'a Workspace> {
    workspaces_on(snapshot, output)
        .into_iter()
        .rev()
        .find(|workspace| workspace.name.is_none() && windows_on(snapshot, workspace.id).is_empty())
}

pub fn space_name(workspace: WorkspaceId) -> String {
    format!("{SPACE_NAME_PREFIX}{}", workspace.0)
}

/// What + does: name niri's spare workspace so it stays as a new Space. When
/// the user is standing on the spare workspace it is already listed, so it
/// is pinned and the caller names the next spare one once niri creates it
/// (`Added::PinnedCurrent`).
#[derive(Clone, Debug, PartialEq)]
pub enum Added {
    New(Action),
    PinnedCurrent(Action),
}

pub fn add_space(snapshot: &Snapshot, output: &OutputId) -> Option<Added> {
    let spare = spare_workspace(snapshot, output)?;
    let action = Action::NameWorkspace {
        workspace: spare.id,
        name: space_name(spare.id),
    };
    Some(if spare.active {
        Added::PinnedCurrent(action)
    } else {
        Added::New(action)
    })
}

/// Remove `space`: its windows move to the Space before it (the next one
/// for the first), focus follows if it was current, and its name is dropped
/// so niri deletes the emptied workspace. The last Space cannot be removed.
pub fn remove_space(snapshot: &Snapshot, output: &OutputId, space: WorkspaceId) -> Vec<Action> {
    let spaces = spaces(snapshot, output);
    let Some(index) = spaces
        .iter()
        .position(|candidate| candidate.workspace == space)
    else {
        return Vec::new();
    };
    if spaces.len() < 2 {
        return Vec::new();
    }
    let target = if index == 0 {
        spaces[1].workspace
    } else {
        spaces[index - 1].workspace
    };
    let removed = &spaces[index];
    let mut actions = Vec::new();
    if removed.active {
        actions.push(Action::FocusWorkspace { workspace: target });
    }
    for window in &removed.windows {
        actions.push(Action::MoveWindowToWorkspace {
            window: *window,
            workspace: target,
            follow: false,
        });
    }
    if removed.named {
        actions.push(Action::UnnameWorkspace { workspace: space });
    }
    actions
}

/// ⌃→ / ⌃←: the neighbouring Space on the focused output. It does not wrap,
/// and skips niri's spare and parking workspaces.
pub fn neighbour_space(snapshot: &Snapshot, forward: bool) -> Option<WorkspaceId> {
    let output = focused_output(snapshot)?;
    let spaces = spaces(snapshot, &output);
    let current = spaces.iter().position(|space| space.active)?;
    let next = if forward {
        current.checked_add(1)?
    } else {
        current.checked_sub(1)?
    };
    spaces.get(next).map(|space| space.workspace)
}

// ---------------------------------------------------------------------------
// Show Desktop

/// A shown desktop: the Space the windows are on, and the spare workspace
/// being shown instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShownDesktop {
    pub output: OutputId,
    pub from: WorkspaceId,
    pub empty: WorkspaceId,
}

/// F11: what to show, or `None` when the current Space is already empty.
pub fn show_desktop(snapshot: &Snapshot) -> Option<ShownDesktop> {
    let output = focused_output(snapshot)?;
    let from = active_workspace(snapshot, &output)?;
    if windows_on(snapshot, from.id).is_empty() {
        return None;
    }
    let empty = spare_workspace(snapshot, &output)?;
    Some(ShownDesktop {
        output,
        from: from.id,
        empty: empty.id,
    })
}

/// F11 again: go back only if the user is still looking at the desktop.
pub fn restore_desktop(snapshot: &Snapshot, shown: &ShownDesktop) -> Option<Action> {
    let active = active_workspace(snapshot, &shown.output)?;
    let from_exists = snapshot
        .workspaces
        .iter()
        .any(|workspace| workspace.id == shown.from);
    (active.id == shown.empty && from_exists).then_some(Action::FocusWorkspace {
        workspace: shown.from,
    })
}

// ---------------------------------------------------------------------------
// Hot corners

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub fn action(self, settings: &HotCornerSettings) -> HotCornerAction {
        match self {
            Self::TopLeft => settings.top_left,
            Self::TopRight => settings.top_right,
            Self::BottomLeft => settings.bottom_left,
            Self::BottomRight => settings.bottom_right,
        }
    }
}

/// What a corner does once the pointer reaches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CornerEffect {
    /// Handled by this service.
    Command(Command),
    /// A shell surface reached through `rmac-shortcut-dispatch`.
    Dispatch(&'static str),
}

pub fn corner_effect(action: HotCornerAction) -> Option<CornerEffect> {
    Some(match action {
        HotCornerAction::None => return None,
        HotCornerAction::MissionControl => CornerEffect::Command(Command::MissionControl),
        HotCornerAction::ApplicationWindows => CornerEffect::Command(Command::AppWindows),
        HotCornerAction::Desktop => CornerEffect::Command(Command::ShowDesktop),
        HotCornerAction::NotificationCenter => CornerEffect::Dispatch("notification-center"),
        HotCornerAction::Apps => CornerEffect::Dispatch("app-drawer"),
        HotCornerAction::LockScreen => CornerEffect::Dispatch("lock"),
    })
}

/// Corners that need a surface.
pub fn active_corners(settings: &HotCornerSettings) -> Vec<Corner> {
    Corner::ALL
        .into_iter()
        .filter(|corner| corner.action(settings) != HotCornerAction::None)
        .collect()
}

// ---------------------------------------------------------------------------
// Keyboard selection

/// Arrow-key movement between laid-out windows: the nearest window whose
/// centre lies in the pressed direction.
pub fn step_selection(rects: &[Rect], from: Option<usize>, dx: f32, dy: f32) -> Option<usize> {
    if rects.is_empty() {
        return None;
    }
    let Some(from) = from.filter(|index| *index < rects.len()) else {
        return Some(0);
    };
    let (fx, fy) = rects[from].centre();
    rects
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != from)
        .filter_map(|(index, rect)| {
            let (x, y) = rect.centre();
            let along = (x - fx) * dx + (y - fy) * dy;
            if along <= 0.5 {
                return None;
            }
            let across = ((x - fx) * dy - (y - fy) * dx).abs();
            Some((index, along + 2.0 * across))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
        .or(Some(from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{
        FocusState, LogicalOutput, LogicalPoint, LogicalSize, Output, OutputMode, PhysicalSize,
        WindowLayout,
    };

    fn output() -> Output {
        Output {
            id: OutputId::from("eDP-1"),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: vec![OutputMode {
                physical_size: PhysicalSize {
                    width: 2940,
                    height: 1912,
                },
                refresh_millihz: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(LogicalOutput {
                position: LogicalPoint::default(),
                size: LogicalSize {
                    width: 1470.0,
                    height: 956.0,
                },
                scale: 2.0,
                transform: "normal".into(),
            }),
        }
    }

    fn workspace(id: u64, index: u8, name: Option<&str>, active: bool) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            index,
            name: name.map(str::to_owned),
            output: Some(OutputId::from("eDP-1")),
            urgent: false,
            active,
            focused: active,
            active_window: None,
        }
    }

    fn window(id: u64, app: &str, workspace: u64, frame: Rect, focused_at: u64) -> Window {
        Window {
            id: WindowId(id),
            title: Some(format!("window {id}")),
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
            layout: WindowLayout {
                scrolling_position: None,
                tile_size: LogicalSize {
                    width: f64::from(frame.width),
                    height: f64::from(frame.height),
                },
                tile_position_in_view: Some(LogicalPoint {
                    x: f64::from(frame.x),
                    y: f64::from(frame.y),
                }),
                window_size: PhysicalSize::default(),
                window_offset_in_tile: LogicalPoint::default(),
            },
        }
    }

    /// Desktop (1, current), parking (2), a pinned Space (3), niri's spare (4).
    fn snapshot(windows: Vec<Window>, focused: Option<u64>) -> Snapshot {
        Snapshot {
            outputs: vec![output()],
            workspaces: vec![
                workspace(1, 1, Some("Desktop"), true),
                workspace(2, 2, Some(PARKING_WORKSPACE), false),
                workspace(3, 3, Some("rmac-space-3"), false),
                workspace(4, 4, None, false),
            ],
            windows,
            focus: FocusState {
                output: Some(OutputId::from("eDP-1")),
                workspace: Some(WorkspaceId(1)),
                window: focused.map(WindowId),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn commands_round_trip_and_fit_the_wire() {
        for command in [
            Command::MissionControl,
            Command::AppWindows,
            Command::ShowDesktop,
            Command::NextSpace,
            Command::PreviousSpace,
            Command::Cancel,
        ] {
            assert_eq!(Command::parse(command.as_str()), Some(command));
            assert!(command.as_str().len() <= 16);
        }
        assert_eq!(Command::parse("toggle-overview"), None);
    }

    #[test]
    fn app_expose_layout_lands_on_the_macs_measured_frames() {
        let terminal = Rect::new(0.0, 0.0, 557.0, 370.0);
        let frames = [
            Rect {
                x: 100.0,
                ..terminal
            },
            Rect {
                x: 700.0,
                ..terminal
            },
            Rect {
                x: 400.0,
                y: 300.0,
                ..terminal
            },
        ];
        let placed = layout(
            &frames,
            app_windows_area(1470.0, 956.0),
            GAP_X,
            0.0,
            CAPTION_BAND,
        );
        let rounded: Vec<(f32, f32, f32, f32)> = placed
            .iter()
            .map(|r| (r.x.round(), r.y.round(), r.width.round(), r.height.round()))
            .collect();
        assert_eq!(
            rounded,
            [
                (99.0, 52.0, 557.0, 370.0),
                (814.0, 52.0, 557.0, 370.0),
                (457.0, 454.0, 557.0, 370.0),
            ]
        );
    }

    #[test]
    fn mission_control_never_enlarges_and_keeps_windows_apart() {
        let area = mission_control_area(1470.0, 956.0, BAR_COLLAPSED);
        assert_eq!(area, Rect::new(20.0, 112.0, 1430.0, 740.0));
        let expanded = mission_control_area(1470.0, 956.0, BAR_EXPANDED);
        assert_eq!(expanded, Rect::new(20.0, 204.0, 1430.0, 648.0));

        let one = layout(
            &[Rect::new(10.0, 10.0, 300.0, 200.0)],
            area,
            GAP_X,
            GAP_Y,
            0.0,
        );
        assert_eq!((one[0].width, one[0].height), (300.0, 200.0));

        let frames: Vec<Rect> = (0..7)
            .map(|i| Rect::new(i as f32 * 90.0, i as f32 * 60.0, 900.0, 600.0))
            .collect();
        let placed = layout(&frames, area, GAP_X, GAP_Y, 0.0);
        for (i, a) in placed.iter().enumerate() {
            assert!(a.x >= area.x - 0.01 && a.right() <= area.right() + 0.01);
            assert!(a.y >= area.y - 0.01 && a.bottom() <= area.bottom() + 0.01);
            for b in &placed[i + 1..] {
                assert!(!a.overlaps(b, 0.0), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn scene_lists_the_current_space_bottom_first_and_marks_covered_windows() {
        let snapshot = snapshot(
            vec![
                window(1, "zed", 1, Rect::new(0.0, 40.0, 800.0, 600.0), 10),
                window(2, "files", 1, Rect::new(600.0, 300.0, 500.0, 400.0), 20),
                window(3, "zed", 1, Rect::new(1200.0, 60.0, 200.0, 200.0), 5),
                window(4, "zed", 3, Rect::new(0.0, 0.0, 400.0, 300.0), 30),
                window(5, "dev.rmac.Dock", 1, Rect::new(0.0, 0.0, 10.0, 10.0), 40),
            ],
            Some(2),
        );
        let scene = scene(&snapshot, Mode::MissionControl).unwrap();
        assert_eq!(scene.workspace, WorkspaceId(1));
        let ids: Vec<u64> = scene.windows.iter().map(|w| w.id.0).collect();
        assert_eq!(ids, [3, 1, 2]);
        let occluded: Vec<bool> = scene.windows.iter().map(|w| w.occluded).collect();
        assert_eq!(occluded, [false, true, false]);

        let expose = super::scene(&snapshot, Mode::AppWindows).unwrap();
        assert!(expose
            .windows
            .iter()
            .all(|w| w.app_id.as_deref() == Some("files")));
        assert!(
            super::scene(&super::tests::snapshot(Vec::new(), None), Mode::AppWindows).is_none()
        );
    }

    #[test]
    fn spaces_skip_parking_and_niris_spare_workspace() {
        let snapshot = snapshot(
            vec![window(1, "zed", 1, Rect::new(0.0, 0.0, 9.0, 9.0), 1)],
            None,
        );
        let spaces = spaces(&snapshot, &OutputId::from("eDP-1"));
        let labels: Vec<&str> = spaces.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Desktop 1", "Desktop 2"]);
        assert_eq!(spaces[1].workspace, WorkspaceId(3));

        let mut alone = snapshot.clone();
        alone.workspaces.retain(|w| w.id != WorkspaceId(3));
        assert_eq!(
            super::spaces(&alone, &OutputId::from("eDP-1"))[0].label,
            "Desktop"
        );
    }

    #[test]
    fn plus_names_the_spare_workspace_and_remove_moves_windows_back() {
        let output = OutputId::from("eDP-1");
        let mut snapshot = snapshot(
            vec![window(7, "zed", 3, Rect::new(0.0, 0.0, 9.0, 9.0), 1)],
            None,
        );
        assert_eq!(
            add_space(&snapshot, &output),
            Some(Added::New(Action::NameWorkspace {
                workspace: WorkspaceId(4),
                name: "rmac-space-4".into(),
            }))
        );
        assert_eq!(
            remove_space(&snapshot, &output, WorkspaceId(3)),
            [
                Action::MoveWindowToWorkspace {
                    window: WindowId(7),
                    workspace: WorkspaceId(1),
                    follow: false,
                },
                Action::UnnameWorkspace {
                    workspace: WorkspaceId(3),
                },
            ]
        );
        // Standing on the spare workspace pins it first.
        for workspace in &mut snapshot.workspaces {
            workspace.active = workspace.id == WorkspaceId(4);
        }
        assert!(matches!(
            add_space(&snapshot, &output),
            Some(Added::PinnedCurrent(_))
        ));
        // The only Space cannot be removed.
        snapshot.workspaces.retain(|w| w.id != WorkspaceId(3));
        for workspace in &mut snapshot.workspaces {
            workspace.active = workspace.id == WorkspaceId(1);
        }
        assert!(remove_space(&snapshot, &output, WorkspaceId(1)).is_empty());
    }

    #[test]
    fn control_arrows_walk_spaces_without_wrapping() {
        let snapshot = snapshot(Vec::new(), None);
        assert_eq!(neighbour_space(&snapshot, true), Some(WorkspaceId(3)));
        assert_eq!(neighbour_space(&snapshot, false), None);
    }

    #[test]
    fn show_desktop_uses_the_spare_workspace_and_returns_only_from_it() {
        let mut snapshot = snapshot(
            vec![window(1, "zed", 1, Rect::new(0.0, 0.0, 9.0, 9.0), 1)],
            None,
        );
        let shown = show_desktop(&snapshot).unwrap();
        assert_eq!((shown.from, shown.empty), (WorkspaceId(1), WorkspaceId(4)));
        assert_eq!(restore_desktop(&snapshot, &shown), None);
        for workspace in &mut snapshot.workspaces {
            workspace.active = workspace.id == WorkspaceId(4);
        }
        assert_eq!(
            restore_desktop(&snapshot, &shown),
            Some(Action::FocusWorkspace {
                workspace: WorkspaceId(1)
            })
        );
        snapshot.windows.clear();
        for workspace in &mut snapshot.workspaces {
            workspace.active = workspace.id == WorkspaceId(1);
        }
        assert_eq!(show_desktop(&snapshot), None);
    }

    #[test]
    fn hot_corners_map_to_commands_and_dispatches() {
        let settings = HotCornerSettings {
            top_left: HotCornerAction::MissionControl,
            bottom_right: HotCornerAction::LockScreen,
            ..HotCornerSettings::default()
        };
        assert_eq!(
            active_corners(&settings),
            [Corner::TopLeft, Corner::BottomRight]
        );
        assert_eq!(
            corner_effect(HotCornerAction::MissionControl),
            Some(CornerEffect::Command(Command::MissionControl))
        );
        assert_eq!(
            corner_effect(HotCornerAction::Apps),
            Some(CornerEffect::Dispatch("app-drawer"))
        );
        assert_eq!(corner_effect(HotCornerAction::None), None);
    }

    #[test]
    fn easing_and_centres_are_symmetric() {
        assert!(ease(0.0).abs() < 1e-3);
        assert!((ease(1.0) - 1.0).abs() < 1e-3);
        assert!((ease(0.5) - 0.5).abs() < 1e-2);
        assert_eq!(space_centres(1, 1470.0), [735.0]);
        assert_eq!(space_centres(2, 1470.0), [650.0, 820.0]);
    }

    #[test]
    fn arrows_move_to_the_nearest_window_in_that_direction() {
        let rects = [
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Rect::new(200.0, 0.0, 100.0, 100.0),
            Rect::new(0.0, 200.0, 100.0, 100.0),
        ];
        assert_eq!(step_selection(&rects, None, 1.0, 0.0), Some(0));
        assert_eq!(step_selection(&rects, Some(0), 1.0, 0.0), Some(1));
        assert_eq!(step_selection(&rects, Some(0), 0.0, 1.0), Some(2));
        assert_eq!(step_selection(&rects, Some(0), -1.0, 0.0), Some(0));
    }
}
