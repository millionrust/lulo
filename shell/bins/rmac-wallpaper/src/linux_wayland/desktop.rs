//! Desktop items on the wallpaper: Finder's icon grid, selection and the
//! marquee, moving and snapping icons, dropping them on folders, Stacks,
//! desktop widgets, renaming in place, Get Info and View Options. Measured
//! numbers live in rmac_desktop::grid; the rest is marked S in
//! design-lab/desktop.html.

use super::menu::{Command, DesktopMenu, MenuTarget};
use super::*;
use gpui::{Focusable as _, Subscription};
use rmac_desktop::grid::{Grid, Placement, ViewOptions, LABEL_GAP, LABEL_MAX_WIDTH};
use rmac_desktop::rename::{self as naming, NameCheck};
use rmac_desktop::stacks::{self, StackKind, Tile};
use rmac_desktop::widgets::{self as desk_widgets, Widget};
use rmac_desktop::{Item, ItemKind, RenameError};
use rmac_shell_ui::text_field::{TextField, TextFieldEvent, TextFieldStyle};
use std::sync::atomic::AtomicBool;

/// A press that moves further than this starts a drag.
const DRAG_THRESHOLD: f32 = 3.0;
/// Selected icon backdrop (S): 4 outside the icon box, radius 6.
const SELECTION_BACKDROP: u32 = 0x00000059;
const SELECTION_OUTSET: f32 = 4.0;
const SELECTION_RADIUS: f32 = 6.0;
/// Label pill (S): 5 side padding, radius 4; accent while the desktop is
/// focused, grey otherwise.
const LABEL_PAD: f32 = 5.0;
const LABEL_RADIUS: f32 = 4.0;
const LABEL_INACTIVE: u32 = 0xFFFFFF40;
/// Marquee (S).
const MARQUEE_FILL: u32 = 0xFFFFFF1F;
const MARQUEE_BORDER: u32 = 0xFFFFFF80;
/// Image files larger than this show the generic document icon.
const PREVIEW_LIMIT: u64 = 32 * 1024 * 1024;
const PREVIEW_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
/// Info and View Options panels (S).
const PANEL_RADIUS: f32 = 12.0;
const INFO_WIDTH: f32 = 265.0;
const OPTIONS_WIDTH: f32 = 240.0;
const PANEL_TOP: f32 = 60.0;
const PANEL_INSET: f32 = 20.0;
const TRACK_WIDTH: f32 = OPTIONS_WIDTH - 2.0 * PANEL_INSET;
const CLOSE_RED: u32 = 0xFF5F57FF;
/// Rename (S): a second click on a selected icon's label starts editing
/// after this pause, longer than the 400 ms double-click interval, so a
/// double-click still opens. Selected text is white 0.35 over the accent.
const RENAME_DELAY: Duration = Duration::from_millis(500);
const RENAME_SELECTION: u32 = 0xFFFFFF59;
/// Rename alerts (S): a 260-wide card 30% down the screen.
const ALERT_WIDTH: f32 = 260.0;
const ALERT_PADDING: f32 = 20.0;
const ALERT_TOP: f32 = 0.3;
const ALERT_BUTTON: f32 = 28.0;
const ALERT_BUTTON_RADIUS: f32 = 7.0;
const ALERT_BUTTON_FILL: u32 = 0xFFFFFF26;

#[derive(Default)]
pub(crate) struct DeskState {
    pub selection: BTreeSet<PathBuf>,
    pub menu: Option<DesktopMenu>,
    /// Open With ▸ candidates for the single file the open context menu
    /// targets (DESK-01), fetched once when the menu opens
    /// (`Wallpaper::open_context_menu`) — never polled.
    pub open_with: Option<(PathBuf, rmac_apps::FileAssociation)>,
    pub drag: Option<Drag>,
    pub expanded: BTreeSet<StackKind>,
    pub panel: Option<Panel>,
    pub rename: Option<Rename>,
    /// Counts presses and keys, so a pending click-to-rename can tell it
    /// was followed by something else (a double-click opens instead).
    pub rename_click: u64,
    /// The press on bare wallpaper was a plain click that dismissed nothing;
    /// released in place, it shows the desktop (reveal.rs).
    pub reveal_click: bool,
}

/// An icon's name being edited in place.
pub(crate) struct Rename {
    path: PathBuf,
    name: String,
    field: Entity<TextField>,
    _events: Subscription,
    /// The rename is running on the file system.
    pending: bool,
    alert: Option<RenameAlert>,
}

enum RenameAlert {
    Taken(String),
    Invalid(String),
    /// The new name begins with a dot: Cancel or Use ".".
    Hidden,
    Failed(std::io::ErrorKind),
}

/// What a finished rename needs to update the selection and positions.
struct Renamed {
    old_path: PathBuf,
    old_name: String,
    new_name: String,
    /// Every loose icon's spot when the rename started (None when the
    /// desktop is stacked or sorted, where icons don't keep spots).
    placements: Option<Vec<(String, Placement)>>,
}

pub(crate) enum Drag {
    Icons {
        start: Point<Pixels>,
        current: Point<Pixels>,
        moved: bool,
        pressed: PathBuf,
        additive: bool,
        /// A plain click on the label of the only selected icon: renaming
        /// starts after a pause unless another click follows.
        rename_on_release: bool,
    },
    Marquee {
        start: Point<Pixels>,
        current: Point<Pixels>,
        base: BTreeSet<PathBuf>,
    },
    Widget {
        id: u64,
        origin: (f32, f32),
        start: Point<Pixels>,
        current: Point<Pixels>,
    },
    Slider {
        control: SliderControl,
        track_left: f32,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum SliderControl {
    IconSize,
    GridSpacing,
}

#[derive(Clone)]
pub(crate) enum Panel {
    /// `None` is the Desktop folder itself.
    Info(Option<PathBuf>),
    ViewOptions,
}

struct Placed {
    tile: Tile,
    left: f32,
    top: f32,
}

pub(crate) struct DeskLayout {
    grid: Grid,
    items: Vec<Item>,
    placed: Vec<Placed>,
}

impl DeskLayout {
    fn item(&self, placed: &Placed) -> Option<&Item> {
        match placed.tile {
            Tile::Item { index, .. } => self.items.get(index),
            Tile::Stack { .. } => None,
        }
    }

    /// Icon box plus the label's two lines, centred on the icon.
    fn bounds(&self, placed: &Placed) -> (f32, f32, f32, f32) {
        let icon = self.grid.options.icon_size;
        let centre = placed.left + icon / 2.0;
        let width = LABEL_MAX_WIDTH.max(icon);
        (
            centre - width / 2.0,
            placed.top,
            width,
            icon + LABEL_GAP + 2.0 * self.grid.options.label_line(),
        )
    }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.placed.iter().rposition(|placed| {
            let (left, top, width, height) = self.bounds(placed);
            x >= left && x < left + width && y >= top && y < top + height
        })
    }

    /// Where every loose item is now, by name.
    fn item_placements(&self) -> Vec<(String, Placement)> {
        self.placed
            .iter()
            .filter_map(|placed| {
                self.item(placed).map(|item| {
                    (
                        item.name.clone(),
                        self.grid.placement_at(placed.left, placed.top),
                    )
                })
            })
            .collect()
    }
}

/// The Dock's exclusive zone, which the icon grid stays above.
fn dock_reserved() -> f32 {
    tokens::dock_tile() * (1.0 + 2.0 * 0.15625 + 0.078125)
}

fn rect_from(start: Point<Pixels>, current: Point<Pixels>) -> (f32, f32, f32, f32) {
    let (x0, y0) = (f32::from(start.x), f32::from(start.y));
    let (x1, y1) = (f32::from(current.x), f32::from(current.y));
    (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
}

fn intersects(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

fn format_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", units[unit])
    } else {
        format!("{value:.0} {}", units[unit])
    }
}

fn item_icon(item: &Item, size: f32) -> AnyElement {
    if item.kind == ItemKind::Directory {
        return img(FOLDER_ICON).size(px(size)).into_any_element();
    }
    let extension = extension(&item.name);
    if PREVIEW_EXTENSIONS.contains(&extension.as_str()) && item.size_bytes <= PREVIEW_LIMIT {
        return img(item.path.clone())
            .size(px(size))
            .object_fit(gpui::ObjectFit::Contain)
            .into_any_element();
    }
    div()
        .relative()
        .size(px(size))
        .child(img(DOCUMENT_ICON).size(px(size)))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(size * 0.19))
                .flex()
                .justify_center()
                .text_size(px(size * 0.125))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(0x6B6B73FF))
                .child(extension.chars().take(4).collect::<String>().to_uppercase()),
        )
        .into_any_element()
}

/// A stack: its newest items piled with a slight offset (S).
fn stack_icon<'a>(members: impl Iterator<Item = &'a Item>, size: f32) -> AnyElement {
    let layer = size * 0.8;
    let members = members.take(3).collect::<Vec<_>>();
    let count = members.len();
    div()
        .relative()
        .size(px(size))
        .children(members.into_iter().enumerate().rev().map(|(depth, item)| {
            let offset = (count - 1 - depth) as f32 * 3.0;
            div()
                .absolute()
                .left(px((size - layer) / 2.0 - offset + 3.0))
                .top(px((size - layer) / 2.0 - offset + 3.0))
                .child(item_icon(item, layer))
        }))
        .into_any_element()
}

fn panel_card(width: f32) -> gpui::Div {
    div()
        .absolute()
        .top(px(PANEL_TOP))
        .w(px(width))
        .rounded(px(PANEL_RADIUS))
        .bg(rgba(tokens::regular_dark_tint()))
        .border_1()
        .border_color(rgba(tokens::light_border()))
        .shadow_lg()
        .text_size(px(13.0))
        .text_color(rgba(tokens::primary_text()))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

impl Wallpaper {
    fn desk_layout(&self, window: &Window, cx: &App) -> DeskLayout {
        let size = window.viewport_size();
        let status = self.status.read(cx);
        let items = status
            .desktop
            .as_ref()
            .map(|snapshot| snapshot.items.clone())
            .unwrap_or_default();
        let settings = &status.settings;
        let grid = Grid::new(
            f32::from(size.width),
            f32::from(size.height),
            dock_reserved(),
            settings.view,
        );
        let (tiles, placements) = if settings.use_stacks {
            let tiles = stacks::tiles(&stacks::group(&items), &self.desk.expanded);
            let placements = grid.arrange(tiles.len());
            (tiles, placements)
        } else {
            let tiles = (0..items.len())
                .map(|index| Tile::Item { index, stack: None })
                .collect::<Vec<_>>();
            let placements = if settings.arrangement.is_sorted() {
                grid.arrange(items.len())
            } else {
                grid.layout(
                    items.iter().map(|item| item.name.as_str()),
                    &settings.positions,
                )
            };
            (tiles, placements)
        };
        let placed = tiles
            .into_iter()
            .zip(placements)
            .map(|(tile, placement)| Placed {
                tile,
                left: grid.left(placement),
                top: placement.top,
            })
            .collect();
        DeskLayout {
            grid,
            items,
            placed,
        }
    }

    fn open_context_menu(
        &mut self,
        target: MenuTarget,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_display(cx);
        self.dismiss_app_drawer(cx);
        self.desk.drag = None;
        self.desk.rename_click = self.desk.rename_click.wrapping_add(1);
        self.desk.open_with = None;
        // Open With ▸ (DESK-01) needs an XDG MIME lookup, which is async, so
        // it is fetched once here rather than blocking the menu open. A
        // single regular file matches Finder's own Open With scope.
        if let MenuTarget::Items(paths) = &target {
            if let [path] = paths.as_slice() {
                if path.is_file() {
                    let path = path.clone();
                    cx.spawn(async move |this, cx| {
                        if let Ok(association) =
                            rmac_app_launch::file_association(path.clone()).await
                        {
                            let _ = this.update(cx, |this, cx| {
                                this.desk.open_with = Some((path, association));
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
            }
        }
        self.desk.menu = Some(DesktopMenu::new(position, target));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn background_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_display(cx);
        // A second press of a double-click is not a new click (S).
        self.desk.reveal_click = event.click_count == 1
            && !(event.modifiers.platform || event.modifiers.shift)
            && self.desk.menu.is_none()
            && self.desk.panel.is_none()
            && self.desk.rename.is_none()
            && self.status.read(cx).app_drawer_window().is_none();
        self.dismiss_app_drawer(cx);
        // Focusing the desktop ends any rename in progress (it commits).
        window.focus(&self.focus, cx);
        self.desk.menu = None;
        self.desk.rename_click = self.desk.rename_click.wrapping_add(1);
        self.action_error = None;
        let additive = event.modifiers.platform || event.modifiers.shift;
        let base = if additive {
            self.desk.selection.clone()
        } else {
            BTreeSet::new()
        };
        self.desk.selection = base.clone();
        self.desk.drag = Some(Drag::Marquee {
            start: event.position,
            current: event.position,
            base,
        });
        cx.notify();
    }

    pub(crate) fn background_context_menu(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.desk.selection.clear();
        self.open_context_menu(MenuTarget::Background, event.position, window, cx);
    }

    fn tile_mouse_down(
        &mut self,
        index: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_display(cx);
        self.dismiss_app_drawer(cx);
        let was_active = self.focus.contains_focused(window, cx);
        window.focus(&self.focus, cx);
        self.desk.menu = None;
        self.desk.rename_click = self.desk.rename_click.wrapping_add(1);
        self.action_error = None;
        let layout = self.desk_layout(window, cx);
        let Some(placed) = layout.placed.get(index) else {
            return;
        };
        match &placed.tile {
            // A click opens or closes a stack in place.
            Tile::Stack { kind, .. } => {
                if !self.desk.expanded.remove(kind) {
                    self.desk.expanded.insert(*kind);
                }
                self.desk.selection.clear();
            }
            Tile::Item { index, .. } => {
                let Some(item) = layout.items.get(*index) else {
                    return;
                };
                let path = item.path.clone();
                if event.click_count >= 2 {
                    self.desk.selection.insert(path);
                    self.run_command(Command::Open, None, window, cx);
                    return;
                }
                let additive = event.modifiers.platform || event.modifiers.shift;
                let on_label =
                    f32::from(event.position.y) >= placed.top + layout.grid.options.icon_size;
                let rename_on_release = event.click_count == 1
                    && !additive
                    && was_active
                    && on_label
                    && self.desk.rename.is_none()
                    && self.desk.selection.len() == 1
                    && self.desk.selection.contains(&path);
                if additive {
                    if !self.desk.selection.remove(&path) {
                        self.desk.selection.insert(path.clone());
                    }
                } else if !self.desk.selection.contains(&path) {
                    self.desk.selection = BTreeSet::from([path.clone()]);
                }
                self.desk.drag = Some(Drag::Icons {
                    start: event.position,
                    current: event.position,
                    moved: false,
                    pressed: path,
                    additive,
                    rename_on_release,
                });
            }
        }
        cx.notify();
    }

    fn tile_context_menu(
        &mut self,
        index: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = self.desk_layout(window, cx);
        let path = layout
            .placed
            .get(index)
            .and_then(|placed| layout.item(placed))
            .map(|item| item.path.clone());
        let Some(path) = path else {
            self.desk.selection.clear();
            self.open_context_menu(MenuTarget::Background, event.position, window, cx);
            return;
        };
        if !self.desk.selection.contains(&path) {
            self.desk.selection = BTreeSet::from([path]);
        }
        let paths = self.desk.selection.iter().cloned().collect();
        self.open_context_menu(MenuTarget::Items(paths), event.position, window, cx);
    }

    pub(crate) fn pointer_moved(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.desk.drag.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            // The button came up somewhere this surface did not see.
            self.desk.drag = None;
            cx.notify();
            return;
        }
        let layout = self.desk_layout(window, cx);
        let mut slider = None;
        match self.desk.drag.as_mut() {
            Some(Drag::Icons {
                start,
                current,
                moved,
                ..
            }) => {
                *current = event.position;
                let dx = f32::from(current.x - start.x);
                let dy = f32::from(current.y - start.y);
                if dx.hypot(dy) > DRAG_THRESHOLD {
                    *moved = true;
                }
            }
            Some(Drag::Marquee {
                start,
                current,
                base,
            }) => {
                *current = event.position;
                let rect = rect_from(*start, *current);
                let mut selection = base.clone();
                for placed in &layout.placed {
                    if let Some(item) = layout.item(placed) {
                        if intersects(rect, layout.bounds(placed)) {
                            selection.insert(item.path.clone());
                        }
                    }
                }
                self.desk.selection = selection;
            }
            Some(Drag::Widget { current, .. }) => *current = event.position,
            Some(Drag::Slider {
                control,
                track_left,
            }) => slider = Some((*control, *track_left)),
            None => {}
        }
        if let Some((control, track_left)) = slider {
            self.set_slider(control, f32::from(event.position.x) - track_left, cx);
        }
        cx.notify();
    }

    pub(crate) fn pointer_released(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.desk.drag.take() else {
            return;
        };
        match drag {
            Drag::Icons {
                start,
                moved,
                pressed,
                additive,
                rename_on_release,
                ..
            } => {
                if !moved {
                    if rename_on_release {
                        self.schedule_rename(pressed.clone(), window, cx);
                    }
                    if !additive {
                        self.desk.selection = BTreeSet::from([pressed]);
                    }
                } else {
                    let delta = (
                        f32::from(event.position.x - start.x),
                        f32::from(event.position.y - start.y),
                    );
                    self.drop_icons(delta, event.position, window, cx);
                }
            }
            Drag::Widget {
                id,
                origin,
                start,
                current: _,
            } => {
                let dx = f32::from(event.position.x - start.x);
                let dy = f32::from(event.position.y - start.y);
                if dx.hypot(dy) > DRAG_THRESHOLD {
                    let size = window.viewport_size();
                    let screen = (f32::from(size.width), f32::from(size.height));
                    self.status.update(cx, |status, cx| {
                        status.update_settings(cx, |settings| {
                            if let Some(widget) = settings.widget_mut(id) {
                                let (left, top) = desk_widgets::clamp_origin(
                                    widget.size,
                                    origin.0 + dx,
                                    origin.1 + dy,
                                    screen,
                                );
                                widget.location = WidgetLocation::Desktop { left, top };
                            }
                        });
                    });
                }
            }
            Drag::Marquee { start, .. } => {
                let dx = f32::from(event.position.x - start.x);
                let dy = f32::from(event.position.y - start.y);
                if std::mem::take(&mut self.desk.reveal_click) && dx.hypot(dy) <= DRAG_THRESHOLD {
                    super::reveal::wallpaper_clicked();
                }
            }
            Drag::Slider { .. } => {}
        }
        cx.notify();
    }

    /// Finishes moving the selection: into a folder when dropped on one,
    /// otherwise to the new spot (snapped when Sort By is Snap to Grid).
    /// Stacked and sorted desktops keep their arrangement, as on the Mac.
    fn drop_icons(
        &mut self,
        delta: (f32, f32),
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = self.desk_layout(window, cx);
        let target = layout
            .hit(f32::from(position.x), f32::from(position.y))
            .and_then(|index| layout.item(&layout.placed[index]))
            .filter(|item| {
                item.kind == ItemKind::Directory && !self.desk.selection.contains(&item.path)
            })
            .map(|item| item.path.clone());
        if let Some(folder) = target {
            let paths = self.desk.selection.iter().cloned().collect::<Vec<_>>();
            cx.spawn(async move |this, cx| {
                let result = blocking::unblock(move || {
                    for path in paths {
                        let Some(name) = path.file_name() else {
                            continue;
                        };
                        let destination = folder.join(name);
                        if fs::symlink_metadata(&destination).is_ok() {
                            return Err(());
                        }
                        fs::rename(&path, &destination).map_err(|_| ())?;
                    }
                    Ok(())
                })
                .await;
                let _ = this.update(cx, |this, cx| {
                    if result.is_err() {
                        this.action_error =
                            Some("Some items could not be moved into the folder".into());
                    }
                    this.desk.selection.clear();
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let (use_stacks, arrangement) = {
            let settings = &self.status.read(cx).settings;
            (settings.use_stacks, settings.arrangement)
        };
        if use_stacks || arrangement.is_sorted() {
            return;
        }
        let grid = layout.grid;
        let mut placements = layout.item_placements();
        let mut moved = Vec::new();
        for (index, placed) in layout.placed.iter().enumerate() {
            let Some(item) = layout.item(placed) else {
                continue;
            };
            if self.desk.selection.contains(&item.path) {
                if let Some(entry) = placements.iter_mut().find(|(name, _)| *name == item.name) {
                    entry.1 =
                        grid.clamp(grid.placement_at(placed.left + delta.0, placed.top + delta.1));
                    moved.push(index);
                }
            }
        }
        if moved.is_empty() {
            return;
        }
        if arrangement == Arrangement::SnapToGrid {
            let cleaned = grid.clean_up(
                &placements
                    .iter()
                    .map(|(_, placement)| *placement)
                    .collect::<Vec<_>>(),
            );
            for (entry, placement) in placements.iter_mut().zip(cleaned) {
                entry.1 = placement;
            }
        }
        self.save_positions(placements, cx);
    }

    fn save_positions(&mut self, placements: Vec<(String, Placement)>, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            status.update_settings(cx, |settings| {
                settings.positions = placements.into_iter().collect();
            });
        });
    }

    fn set_slider(&mut self, control: SliderControl, offset: f32, cx: &mut Context<Self>) {
        let fraction = (offset / TRACK_WIDTH).clamp(0.0, 1.0);
        self.status.update(cx, |status, cx| {
            status.update_settings(cx, |settings| match control {
                SliderControl::IconSize => {
                    let range = ViewOptions::ICON_SIZES;
                    let value = range.start() + fraction * (range.end() - range.start());
                    // Finder's icon size moves in steps of 4.
                    settings.view.icon_size = (value / 4.0).round() * 4.0;
                }
                SliderControl::GridSpacing => {
                    let range = ViewOptions::GRID_SPACINGS;
                    settings.view.grid_spacing =
                        (range.start() + fraction * (range.end() - range.start())).round();
                }
            });
        });
    }

    pub(crate) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        self.desk.rename_click = self.desk.rename_click.wrapping_add(1);
        if self.desk.menu.is_some() {
            self.menu_key(key, window, cx);
            return true;
        }
        if let Some(rename) = &self.desk.rename {
            if rename.alert.is_none() {
                // The rename field handles its own keys.
                return false;
            }
            // Return picks the default button and Escape cancels; both
            // are the first button.
            if matches!(key, "enter" | "escape") {
                self.rename_alert_choice(false, window, cx);
            }
            return true;
        }
        let modifiers = &event.keystroke.modifiers;
        let command = modifiers.platform;
        match key {
            "enter" if !command && self.desk.selection.len() == 1 => {
                if let Some(path) = self.desk.selection.iter().next().cloned() {
                    self.begin_rename(path, window, cx);
                }
            }
            "escape" => {
                if self.desk.panel.take().is_none() {
                    self.desk.selection.clear();
                }
            }
            "a" if command => {
                self.desk.selection = self
                    .status
                    .read(cx)
                    .desktop
                    .as_ref()
                    .map(|snapshot| {
                        snapshot
                            .items
                            .iter()
                            .map(|item| item.path.clone())
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "backspace" if command => self.run_command(Command::MoveToTrash, None, window, cx),
            "o" | "down" if command => self.run_command(Command::Open, None, window, cx),
            "i" if command => self.run_command(Command::GetInfo, None, window, cx),
            "d" if command => self.run_command(Command::Duplicate, None, window, cx),
            "n" if command && modifiers.shift => {
                self.run_command(Command::NewFolder, None, window, cx)
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn run_command(
        &mut self,
        command: Command,
        target: Option<MenuTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = match &target {
            Some(MenuTarget::Items(paths)) => paths.clone(),
            Some(MenuTarget::Background) => Vec::new(),
            _ => self.desk.selection.iter().cloned().collect::<Vec<_>>(),
        };
        match command {
            Command::NewFolder => {
                let directory = self
                    .status
                    .read(cx)
                    .desktop
                    .as_ref()
                    .map(|snapshot| snapshot.directory.clone());
                let Some(directory) = directory else {
                    self.action_error = Some("The Desktop directory is unavailable".into());
                    return;
                };
                cx.spawn(async move |this, cx| {
                    let result =
                        blocking::unblock(move || rmac_desktop::create_folder(&directory)).await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(path) => this.desk.selection = BTreeSet::from([path]),
                            Err(_) => {
                                this.action_error =
                                    Some("The new folder could not be created".into())
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Command::GetInfo => {
                self.desk.panel = Some(Panel::Info(selection.first().cloned()));
            }
            Command::ChangeWallpaper => spawn_settings("wallpaper", cx),
            Command::EditWidgets => {
                let status = self.status.clone();
                let display = Some(self.display);
                App::defer(cx, move |cx| {
                    gallery::open(status, GalleryTarget::Desktop, display, cx);
                });
            }
            Command::ToggleStacks => {
                self.desk.expanded.clear();
                self.desk.selection.clear();
                self.status.update(cx, |status, cx| {
                    status.update_settings(cx, |settings| {
                        settings.use_stacks = !settings.use_stacks;
                    });
                });
            }
            Command::CleanUp => {
                let Some(layout) = Some(self.desk_layout(window, cx)) else {
                    return;
                };
                let placements = layout.item_placements();
                let cleaned = layout.grid.clean_up(
                    &placements
                        .iter()
                        .map(|(_, placement)| *placement)
                        .collect::<Vec<_>>(),
                );
                let positions = placements
                    .into_iter()
                    .map(|(name, _)| name)
                    .zip(cleaned)
                    .collect();
                self.save_positions(positions, cx);
            }
            Command::CleanUpBy(order) => {
                let Some(layout) = Some(self.desk_layout(window, cx)) else {
                    return;
                };
                let mut items = layout.items.clone();
                rmac_desktop::sort_items(&mut items, order);
                let positions = items
                    .into_iter()
                    .map(|item| item.name)
                    .zip(layout.grid.arrange(layout.items.len()))
                    .collect();
                self.save_positions(positions, cx);
            }
            Command::Arrange(arrangement) => {
                let current = self.status.read(cx).settings.arrangement;
                // Leaving a sorted arrangement keeps the icons where they
                // are; Snap to Grid snaps them.
                let layout = Some(self.desk_layout(window, cx));
                self.status.update(cx, |status, cx| {
                    status.update_settings(cx, |settings| {
                        if let Some(layout) = &layout {
                            if current.is_sorted() && !arrangement.is_sorted() {
                                settings.positions = layout.item_placements().into_iter().collect();
                            }
                            if arrangement == Arrangement::SnapToGrid {
                                let placements = layout.item_placements();
                                let cleaned = layout.grid.clean_up(
                                    &placements
                                        .iter()
                                        .map(|(_, placement)| *placement)
                                        .collect::<Vec<_>>(),
                                );
                                settings.positions = placements
                                    .into_iter()
                                    .map(|(name, _)| name)
                                    .zip(cleaned)
                                    .collect();
                            }
                        }
                        settings.arrangement = arrangement;
                    });
                });
            }
            Command::ViewOptions => self.desk.panel = Some(Panel::ViewOptions),
            Command::Open => {
                for path in selection {
                    spawn_item_action(path, ItemAction::Open, cx);
                }
            }
            Command::OpenWith {
                mime_type,
                application_id,
            } => {
                if let [path] = selection.as_slice() {
                    let path = path.clone();
                    cx.spawn(async move |this, cx| {
                        let result = rmac_app_launch::open_file_with(
                            path,
                            mime_type,
                            application_id,
                            false,
                            false,
                        )
                        .await;
                        let _ = this.update(cx, |this, cx| {
                            if result.is_err() {
                                this.action_error = Some("The item could not be opened".into());
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
            Command::Compress => {
                if selection.is_empty() {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = blocking::unblock(move || {
                        let cancel = AtomicBool::new(false);
                        rmac_archive::compress(&selection, &cancel, &mut |_| {})
                    })
                    .await;
                    let _ = this.update(cx, |this, cx| {
                        if result.is_err() {
                            this.action_error = Some("The items could not be compressed".into());
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Command::MakeAlias => {
                if let [path] = selection.as_slice() {
                    let path = path.clone();
                    cx.spawn(async move |this, cx| {
                        let result =
                            blocking::unblock(move || rmac_desktop::make_alias(&path)).await;
                        let _ = this.update(cx, |this, cx| {
                            match result {
                                Ok(alias) => this.desk.selection = BTreeSet::from([alias]),
                                Err(_) => {
                                    this.action_error =
                                        Some("The alias could not be created".into())
                                }
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
            Command::QuickLook => {
                if selection.is_empty() {
                    return;
                }
                spawn_quick_look(selection, cx);
            }
            Command::Copy => {
                if selection.is_empty() {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = rmac_pasteboard::write_file_list(selection, false)
                        .wait()
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if result.is_err() {
                            this.action_error = Some("The items could not be copied".into());
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Command::MoveToTrash => {
                if selection.is_empty() {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = blocking::unblock(move || trash::delete_all(&selection)).await;
                    let _ = this.update(cx, |this, cx| {
                        if result.is_err() {
                            this.action_error =
                                Some("The items could not be moved to Trash".into());
                        } else {
                            this.desk.selection.clear();
                            let _ = rmac_sound::play(rmac_sound::Cue::Trash);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Command::Rename => {
                if let [path] = selection.as_slice() {
                    self.begin_rename(path.clone(), window, cx);
                }
            }
            Command::Duplicate => {
                if selection.is_empty() {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = blocking::unblock(move || {
                        selection
                            .iter()
                            .map(|path| rmac_desktop::duplicate_file(path))
                            .collect::<Result<BTreeSet<_>, _>>()
                    })
                    .await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(copies) => this.desk.selection = copies,
                            Err(_) => {
                                this.action_error = Some("Only files can be duplicated here".into())
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Command::RemoveWidget(id) => {
                self.status.update(cx, |status, cx| {
                    status.update_settings(cx, |settings| {
                        settings.remove_widget(id);
                    });
                });
            }
            Command::ShowSubmenu(_) => {}
        }
    }

    pub(crate) fn render_desktop(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let layout = self.desk_layout(window, cx);
        let (widgets, data) = {
            let status = self.status.read(cx);
            (
                status
                    .settings
                    .widgets
                    .iter()
                    .filter(|widget| matches!(widget.location, WidgetLocation::Desktop { .. }))
                    .copied()
                    .collect::<Vec<Widget>>(),
                status.widgets.clone(),
            )
        };
        let active = self.focus.contains_focused(window, cx);
        let mut children = Vec::new();
        for widget in widgets {
            children.push(self.render_widget(widget, &data, cx));
        }
        // The icon being renamed is drawn last, so its field's extra lines
        // lie over the icons below it.
        let renaming = self.desk.rename.as_ref().and_then(|rename| {
            layout.placed.iter().position(|placed| {
                layout
                    .item(placed)
                    .is_some_and(|item| item.path == rename.path)
            })
        });
        let order = (0..layout.placed.len())
            .filter(|index| Some(*index) != renaming)
            .chain(renaming);
        for index in order {
            let placed = &layout.placed[index];
            let visual = self.tile_visual(placed, &layout, active, true);
            children.push(
                visual
                    .id(("desktop-tile", index))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.tile_mouse_down(index, event, window, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.tile_context_menu(index, event, window, cx);
                        }),
                    )
                    .into_any_element(),
            );
        }
        match &self.desk.drag {
            Some(Drag::Icons {
                start,
                current,
                moved: true,
                ..
            }) => {
                let dx = f32::from(current.x - start.x);
                let dy = f32::from(current.y - start.y);
                for placed in &layout.placed {
                    let selected = layout
                        .item(placed)
                        .is_some_and(|item| self.desk.selection.contains(&item.path));
                    if !selected {
                        continue;
                    }
                    let ghost = Placed {
                        tile: placed.tile.clone(),
                        left: placed.left + dx,
                        top: placed.top + dy,
                    };
                    children.push(
                        self.tile_visual(&ghost, &layout, active, false)
                            .opacity(0.6)
                            .into_any_element(),
                    );
                }
            }
            Some(Drag::Marquee { start, current, .. }) => {
                let (left, top, width, height) = rect_from(*start, *current);
                children.push(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(top))
                        .w(px(width))
                        .h(px(height))
                        .bg(rgba(MARQUEE_FILL))
                        .border_1()
                        .border_color(rgba(MARQUEE_BORDER))
                        .into_any_element(),
                );
            }
            _ => {}
        }
        if let Some(panel) = self.desk.panel.clone() {
            let element = match panel {
                Panel::Info(path) => self.render_info(path.as_ref(), &layout, window, cx),
                Panel::ViewOptions => Some(self.render_view_options(window, cx)),
            };
            children.extend(element);
        }
        if let Some(menu) = self.render_menu(window, cx) {
            children.push(menu);
        }
        children.extend(self.render_rename_alert(window, cx));
        children
    }

    fn render_widget(&self, widget: Widget, data: &WidgetData, cx: &Context<Self>) -> AnyElement {
        let WidgetLocation::Desktop { left, top } = widget.location else {
            return div().into_any_element();
        };
        let (mut shown_left, mut shown_top) = (left, top);
        if let Some(Drag::Widget {
            id, start, current, ..
        }) = &self.desk.drag
        {
            if *id == widget.id {
                shown_left += f32::from(current.x - start.x);
                shown_top += f32::from(current.y - start.y);
            }
        }
        let id = widget.id;
        div()
            .id(("desktop-widget", id as usize))
            .absolute()
            .left(px(shown_left))
            .top(px(shown_top))
            .child(rmac_desktop_widgets::face(
                widget.kind,
                widget.size,
                1.0,
                data,
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.note_display(cx);
                    window.focus(&this.focus, cx);
                    this.desk.menu = None;
                    this.desk.drag = Some(Drag::Widget {
                        id,
                        origin: (left, top),
                        start: event.position,
                        current: event.position,
                    });
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_context_menu(MenuTarget::Widget(id), event.position, window, cx);
                }),
            )
            .into_any_element()
    }

    /// An icon and its label; with `editing`, the label of the icon being
    /// renamed is its edit field.
    fn tile_visual(
        &self,
        placed: &Placed,
        layout: &DeskLayout,
        active: bool,
        editing: bool,
    ) -> gpui::Div {
        let options = layout.grid.options;
        let icon = options.icon_size;
        let pitch = options.pitch();
        let (label, glyph, selected): (String, AnyElement, bool) = match &placed.tile {
            Tile::Stack { kind, top, .. } => (
                kind.label().to_owned(),
                stack_icon(
                    top.iter().filter_map(|index| layout.items.get(*index)),
                    icon,
                ),
                false,
            ),
            Tile::Item { index, .. } => match layout.items.get(*index) {
                Some(item) => (
                    item.name.clone(),
                    item_icon(item, icon),
                    self.desk.selection.contains(&item.path),
                ),
                None => (String::new(), div().into_any_element(), false),
            },
        };
        let field = match (&placed.tile, &self.desk.rename) {
            (Tile::Item { index, .. }, Some(rename)) if editing => layout
                .items
                .get(*index)
                .filter(|item| item.path == rename.path)
                .map(|_| rename.field.clone()),
            _ => None,
        };
        div()
            .absolute()
            .left(px(placed.left + icon / 2.0 - pitch / 2.0))
            .top(px(placed.top))
            .w(px(pitch))
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .relative()
                    .size(px(icon))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |icon_box| {
                        icon_box.child(
                            div()
                                .absolute()
                                .left(px(-SELECTION_OUTSET))
                                .top(px(-SELECTION_OUTSET))
                                .size(px(icon + 2.0 * SELECTION_OUTSET))
                                .rounded(px(SELECTION_RADIUS))
                                .bg(rgba(SELECTION_BACKDROP)),
                        )
                    })
                    .child(glyph),
            )
            .child(match field {
                Some(field) => div().mt(px(LABEL_GAP)).child(field).into_any_element(),
                None => div()
                    .mt(px(LABEL_GAP))
                    .max_w(px(LABEL_MAX_WIDTH))
                    .px(px(LABEL_PAD))
                    .rounded(px(LABEL_RADIUS))
                    .text_size(px(options.text_size))
                    .line_height(px(options.label_line()))
                    .text_center()
                    .line_clamp(2)
                    .text_color(rgba(0xFFFFFFFF))
                    .when(selected, |label| {
                        label.bg(rgba(if active {
                            tokens::accent()
                        } else {
                            LABEL_INACTIVE
                        }))
                    })
                    .child(label)
                    .into_any_element(),
            })
    }

    /// Clicking a selected icon's label again: rename after a pause, unless
    /// another click or key comes first.
    fn schedule_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.desk.rename_click = self.desk.rename_click.wrapping_add(1);
        let click = self.desk.rename_click;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(RENAME_DELAY).await;
            // An error only means the desktop closed meanwhile.
            this.update_in(cx, |this, window, cx| {
                let undisturbed = this.desk.rename_click == click
                    && this.desk.menu.is_none()
                    && this.desk.selection.len() == 1
                    && this.desk.selection.contains(&path);
                if undisturbed {
                    this.begin_rename(path, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Turns the icon's label into an edit field with the name's stem
    /// selected (all of it for folders and names without an extension).
    pub(crate) fn begin_rename(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.desk.rename.is_some() {
            return;
        }
        let layout = self.desk_layout(window, cx);
        let Some(item) = layout.items.iter().find(|item| item.path == path).cloned() else {
            return;
        };
        let options = layout.grid.options;
        let style = TextFieldStyle {
            text_size: options.text_size,
            line_height: options.label_line(),
            wrap_width: LABEL_MAX_WIDTH - 2.0 * LABEL_PAD,
            padding_x: LABEL_PAD,
            radius: LABEL_RADIUS,
            background: tokens::accent(),
            text: 0xFFFFFFFF,
            selection: RENAME_SELECTION,
            caret: 0xFFFFFFFF,
            centered: true,
        };
        let selection = naming::editable_stem(&item.name, item.kind == ItemKind::Directory);
        let field = cx.new(|cx| {
            TextField::new(
                "desktop-rename",
                format!("Rename {}", item.name),
                &item.name,
                selection,
                style,
                window,
                cx,
            )
        });
        let events = cx.subscribe_in(
            &field,
            window,
            |this, _, event: &TextFieldEvent, window, cx| this.rename_event(*event, window, cx),
        );
        let handle = field.read(cx).focus_handle(cx);
        self.desk.menu = None;
        self.desk.drag = None;
        self.desk.selection = BTreeSet::from([path.clone()]);
        self.desk.rename = Some(Rename {
            path,
            name: item.name,
            field,
            _events: events,
            pending: false,
            alert: None,
        });
        window.focus(&handle, cx);
        cx.notify();
    }

    fn rename_event(&mut self, event: TextFieldEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = &self.desk.rename else {
            return;
        };
        // Focus moves to an alert while it shows; that is not a commit.
        if rename.pending || rename.alert.is_some() {
            return;
        }
        match event {
            TextFieldEvent::Cancel => self.end_rename(window, cx),
            TextFieldEvent::Submit | TextFieldEvent::Blur => self.commit_rename(false, window, cx),
        }
    }

    /// Return, a click elsewhere or focus leaving the desktop: check the
    /// name, then rename on a background thread.
    fn commit_rename(
        &mut self,
        hidden_confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rename) = self.desk.rename.as_ref() else {
            return;
        };
        let path = rename.path.clone();
        let old_name = rename.name.clone();
        let new_name = rename.field.read(cx).text().to_owned();
        let alert = match naming::check_name(&old_name, &new_name) {
            NameCheck::Unchanged | NameCheck::Empty => {
                self.end_rename(window, cx);
                return;
            }
            NameCheck::Invalid => Some(RenameAlert::Invalid(new_name.clone())),
            NameCheck::Hidden if !hidden_confirmed => Some(RenameAlert::Hidden),
            NameCheck::Hidden | NameCheck::Valid => None,
        };
        if let Some(alert) = alert {
            self.show_rename_alert(alert, window, cx);
            return;
        }
        // Where every loose icon is now, so the renamed one keeps its spot.
        let free = {
            let settings = &self.status.read(cx).settings;
            !settings.use_stacks && !settings.arrangement.is_sorted()
        };
        let placements = if free {
            Some(self.desk_layout(window, cx).item_placements())
        } else {
            None
        };
        if let Some(rename) = &mut self.desk.rename {
            rename.pending = true;
        }
        let (old_path, target) = (path.clone(), new_name.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = blocking::unblock(move || rmac_desktop::rename_item(&path, &target)).await;
            let finished = this.update_in(cx, |this, window, cx| {
                let renamed = Renamed {
                    old_path,
                    old_name,
                    new_name,
                    placements,
                };
                this.finish_rename(result, renamed, window, cx)
            });
            if finished.is_err() {
                eprintln!("a Desktop rename finished after the desktop closed");
            }
        })
        .detach();
        cx.notify();
    }

    fn finish_rename(
        &mut self,
        result: Result<PathBuf, RenameError>,
        renamed: Renamed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(rename) = &mut self.desk.rename {
            rename.pending = false;
        }
        let Renamed {
            old_path,
            old_name,
            new_name,
            placements,
        } = renamed;
        match result {
            Ok(path) => {
                if let Some(mut placements) = placements {
                    for entry in &mut placements {
                        if entry.0 == old_name {
                            entry.0 = new_name.clone();
                        }
                    }
                    self.save_positions(placements, cx);
                }
                // A click elsewhere may have changed the selection since.
                if self.desk.selection.remove(&old_path) {
                    self.desk.selection.insert(path);
                }
                self.end_rename(window, cx);
            }
            Err(RenameError::Taken) => {
                self.show_rename_alert(RenameAlert::Taken(new_name), window, cx)
            }
            Err(RenameError::Invalid) => {
                self.show_rename_alert(RenameAlert::Invalid(new_name), window, cx)
            }
            Err(RenameError::Io(error)) => {
                self.show_rename_alert(RenameAlert::Failed(error), window, cx)
            }
        }
    }

    fn show_rename_alert(
        &mut self,
        alert: RenameAlert,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rename) = &mut self.desk.rename else {
            return;
        };
        rename.alert = Some(alert);
        // Keys go to the alert while it shows.
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// OK or Cancel keeps editing (Finder lets you fix the name); Use "."
    /// renames; after a failure, OK ends the rename.
    fn rename_alert_choice(&mut self, proceed: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(alert) = self
            .desk
            .rename
            .as_mut()
            .and_then(|rename| rename.alert.take())
        else {
            return;
        };
        match alert {
            RenameAlert::Hidden if proceed => self.commit_rename(true, window, cx),
            RenameAlert::Failed(_) => self.end_rename(window, cx),
            _ => {
                if let Some(rename) = &self.desk.rename {
                    let handle = rename.field.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                }
            }
        }
        cx.notify();
    }

    fn end_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.desk.rename.take().is_some() {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn render_rename_alert(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let rename = self.desk.rename.as_ref()?;
        let alert = rename.alert.as_ref()?;
        let (title, body) = match alert {
            RenameAlert::Taken(name) => (naming::taken_message(name), None),
            RenameAlert::Invalid(name) => {
                let (title, body) = naming::invalid_message(name);
                (title, Some(body))
            }
            RenameAlert::Hidden => (naming::HIDDEN_TITLE.to_owned(), Some(naming::HIDDEN_BODY)),
            RenameAlert::Failed(error) => {
                let (title, body) = naming::failed_message(&rename.name, *error);
                (title, Some(body))
            }
        };
        // The first button is the default (Return and Escape).
        let buttons: &[(&'static str, bool)] = match alert {
            RenameAlert::Hidden => &[("Cancel", false), ("Use “.”", true)],
            _ => &[("OK", false)],
        };
        let size = window.viewport_size();
        let left = ((f32::from(size.width) - ALERT_WIDTH) / 2.0).max(8.0);
        let top = (f32::from(size.height) * ALERT_TOP).round();
        let buttons = buttons.iter().enumerate().map(|(index, (label, proceed))| {
            let proceed = *proceed;
            div()
                .id(("desktop-rename-alert-button", index))
                .role(Role::Button)
                .aria_label(*label)
                .flex_1()
                .h(px(ALERT_BUTTON))
                .rounded(px(ALERT_BUTTON_RADIUS))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(if index == 0 {
                    tokens::accent()
                } else {
                    ALERT_BUTTON_FILL
                }))
                .child(*label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.rename_alert_choice(proceed, window, cx);
                }))
        });
        Some(
            // Modal: clicks outside the alert do nothing.
            div()
                .id("desktop-rename-alert-scrim")
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("desktop-rename-alert")
                        .role(Role::AlertDialog)
                        .aria_label(SharedString::from(title.clone()))
                        .absolute()
                        .left(px(left))
                        .top(px(top))
                        .w(px(ALERT_WIDTH))
                        .p(px(ALERT_PADDING))
                        .rounded(px(PANEL_RADIUS))
                        .bg(rgba(tokens::regular_dark_tint()))
                        .border_1()
                        .border_color(rgba(tokens::light_border()))
                        .shadow_lg()
                        .text_color(rgba(tokens::primary_text()))
                        .text_size(px(13.0))
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_center()
                        .child(
                            div()
                                .line_height(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .children(body.map(|body| {
                            div()
                                .mt(px(6.0))
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .child(body)
                        }))
                        .child(
                            div()
                                .mt(px(16.0))
                                .w_full()
                                .flex()
                                .gap(px(8.0))
                                .children(buttons),
                        ),
                )
                .into_any_element(),
        )
    }

    fn close_button(id: &'static str, cx: &Context<Self>) -> AnyElement {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label("Close")
            .absolute()
            .left(px(12.0))
            .top(px(12.0))
            .size(px(12.0))
            .rounded_full()
            .bg(rgba(CLOSE_RED))
            .on_click(cx.listener(|this, _, _, cx| {
                this.desk.panel = None;
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_info(
        &self,
        path: Option<&PathBuf>,
        layout: &DeskLayout,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let (title, icon, rows): (String, AnyElement, Vec<(&str, String)>) = match path {
            Some(path) => {
                let item = layout.items.iter().find(|item| &item.path == path)?;
                let kind = if item.kind == ItemKind::Directory {
                    "Folder"
                } else {
                    stacks::kind_label(item)
                };
                let modified = i64::try_from(item.modified_millis)
                    .ok()
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .map(|time| {
                        time.with_timezone(&chrono::Local)
                            .format("%-d %B %Y at %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                let size = if item.kind == ItemKind::Directory {
                    "--".to_owned()
                } else {
                    format_size(item.size_bytes)
                };
                (
                    item.name.clone(),
                    item_icon(item, 32.0),
                    vec![
                        ("Kind:", kind.to_owned()),
                        ("Size:", size),
                        (
                            "Where:",
                            item.path
                                .parent()
                                .map(|parent| parent.display().to_string())
                                .unwrap_or_default(),
                        ),
                        ("Modified:", modified),
                    ],
                )
            }
            None => (
                "Desktop".to_owned(),
                img(FOLDER_ICON).size(px(32.0)).into_any_element(),
                vec![
                    ("Kind:", "Folder".to_owned()),
                    ("Contents:", format!("{} items", layout.items.len())),
                ],
            ),
        };
        let screen_width = f32::from(window.viewport_size().width);
        let left = (screen_width / 2.0 - INFO_WIDTH / 2.0).max(8.0);
        Some(
            panel_card(INFO_WIDTH)
                .id("desktop-info")
                .left(px(left))
                .child(Self::close_button("desktop-info-close", cx))
                .child(
                    div()
                        .h(px(36.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{title} Info")),
                )
                .child(
                    div()
                        .px(px(PANEL_INSET))
                        .pb(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(icon)
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::BOLD)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(
                    div()
                        .px(px(PANEL_INSET))
                        .pb(px(16.0))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .text_size(px(11.0))
                        .children(rows.into_iter().map(|(label, value)| {
                            div()
                                .flex()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .w(px(64.0))
                                        .flex_none()
                                        .flex()
                                        .justify_end()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(label),
                                )
                                .child(div().flex_1().child(value))
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_view_options(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let view = self.status.read(cx).settings.view;
        let screen_width = f32::from(window.viewport_size().width);
        let left = (screen_width - OPTIONS_WIDTH - 80.0).max(8.0);
        let track_left = left + PANEL_INSET;
        let slider = |id: &'static str, control: SliderControl, fraction: f32| {
            let knob = 14.0;
            div()
                .id(id)
                .relative()
                .w(px(TRACK_WIDTH))
                .h(px(20.0))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(8.0))
                        .w(px(TRACK_WIDTH))
                        .h(px(4.0))
                        .rounded(px(2.0))
                        .bg(rgba(0xFFFFFF33)),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(8.0))
                        .w(px(TRACK_WIDTH * fraction))
                        .h(px(4.0))
                        .rounded(px(2.0))
                        .bg(rgba(tokens::accent())),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(TRACK_WIDTH * fraction - knob / 2.0))
                        .top(px(3.0))
                        .size(px(knob))
                        .rounded_full()
                        .bg(rgba(0xFFFFFFFF))
                        .shadow_sm(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.desk.drag = Some(Drag::Slider {
                            control,
                            track_left,
                        });
                        this.set_slider(control, f32::from(event.position.x) - track_left, cx);
                        cx.notify();
                    }),
                )
        };
        let fraction = |value: f32, range: std::ops::RangeInclusive<f32>| {
            ((value - range.start()) / (range.end() - range.start())).clamp(0.0, 1.0)
        };
        let step_button = |id: &'static str, label: &'static str, delta: f32| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(if delta < 0.0 {
                    "Smaller text"
                } else {
                    "Larger text"
                })
                .size(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .bg(rgba(0xFFFFFF1F))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.status.update(cx, |status, cx| {
                        status.update_settings(cx, |settings| {
                            settings.view.text_size += delta;
                        });
                    });
                }))
        };
        let label = |text: String| {
            div()
                .mt(px(10.0))
                .mb(px(2.0))
                .text_size(px(12.0))
                .child(text)
        };
        panel_card(OPTIONS_WIDTH)
            .id("desktop-view-options")
            .left(px(left))
            .child(Self::close_button("desktop-view-options-close", cx))
            .child(
                div()
                    .h(px(36.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Desktop"),
            )
            .child(
                div()
                    .px(px(PANEL_INSET))
                    .pb(px(16.0))
                    .flex()
                    .flex_col()
                    .child(label(format!(
                        "Icon size: {0} × {0}",
                        view.icon_size as u32
                    )))
                    .child(slider(
                        "desktop-icon-size",
                        SliderControl::IconSize,
                        fraction(view.icon_size, ViewOptions::ICON_SIZES),
                    ))
                    .child(label("Grid spacing:".to_owned()))
                    .child(slider(
                        "desktop-grid-spacing",
                        SliderControl::GridSpacing,
                        fraction(view.grid_spacing, ViewOptions::GRID_SPACINGS),
                    ))
                    .child(label("Text size:".to_owned()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(step_button("desktop-text-smaller", "−", -1.0))
                            .child(format!("{} pt", view.text_size as u32))
                            .child(step_button("desktop-text-larger", "+", 1.0)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(12_300), "12 KB");
        assert_eq!(format_size(1_250_000), "1.3 MB");
        assert_eq!(extension("Photo.JPG"), "jpg");
        assert_eq!(extension(".hidden"), "");
    }
}
