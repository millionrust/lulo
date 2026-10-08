//! Lulo mode's desktop (ADR 0023 "Lulo mode"): a window filling the
//! screen just above Explorer's desktop and below every app window, with
//! Lulo's wallpaper and the user's Desktop folder as Lulo icons, laid out,
//! selected, dragged, renamed and opened as on the Mac (the grid and the
//! saved positions are Lulo OS's `rmac-desktop` model). Folders open in
//! Lulo's Files; anything else opens with its Windows app.
//!
//! Nothing here polls: the icons are read again when the Desktop folders
//! report a name change, and the wallpaper when Lulo's settings file
//! changes, the appearance changes or the display does. An idle desktop
//! draws nothing.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    div, img, point, prelude::FluentBuilder as _, px, size, AnyElement, App, AppContext as _,
    Bounds, Context, Entity, FocusHandle, Global, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    ParentElement as _, Pixels, Point, Render, RenderImage, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Window,
};
use rmac_desktop::grid::{Grid, Placement, LABEL_GAP, LABEL_LINES, LABEL_MAX_WIDTH};
use rmac_desktop::settings::{Arrangement, DesktopSettings};
use rmac_ui::{mac, IconSource, InputEvent, InputState, TextField};

use super::{dock, later, runtime, shell, ShellState, BAR_HEIGHT};
use crate::model::menus;
use crate::win::desktop_files::{self, DesktopItem};
use crate::win::{icons, launch, surface, trace, wallpaper, wallpaper_layer, windows_list};

/// A press moves this far before it becomes a drag (as the Mac's 3 pt).
const DRAG_THRESHOLD: f32 = 3.0;
/// The selected icon's backdrop: 4 pt outside the icon, radius 6.
const SELECTION_OUTSET: f32 = 4.0;
const SELECTION_RADIUS: f32 = 6.0;
/// The label pill: 5 pt side padding, radius 4.
const LABEL_PAD: f32 = 5.0;
const LABEL_RADIUS: f32 = 4.0;
/// The Finder's folder artwork for folders; everything else shows its
/// Windows icon (an app shortcut's own icon, a picture's thumbnail).
const FOLDER_ICON: &str = "desktop/folder.svg";

/// One icon as laid out: the item and its icon box's top-left corner.
#[derive(Clone, Debug)]
struct Placed {
    index: usize,
    left: f32,
    top: f32,
}

enum PressKind {
    /// A press on an icon: a drag once it moves far enough.
    Icons { dragging: bool },
    /// A press on the wallpaper: a selection rectangle, adding to `base`.
    Marquee { base: BTreeSet<PathBuf> },
}

struct Press {
    start: Point<Pixels>,
    current: Point<Pixels>,
    kind: PressKind,
}

struct Rename {
    item: DesktopItem,
    field: Entity<InputState>,
    _subscription: Subscription,
}

pub(crate) struct DesktopView {
    shell: Entity<ShellState>,
    focus: FocusHandle,
    items: Vec<DesktopItem>,
    settings: DesktopSettings,
    selection: BTreeSet<PathBuf>,
    press: Option<Press>,
    rename: Option<Rename>,
    active: bool,
    wallpaper: Option<Arc<RenderImage>>,
    /// The wallpaper as a layered child window under the icons
    /// (`win::wallpaper_layer`); `wallpaper` is only the fallback when the
    /// layer cannot show it.
    layer: Option<wallpaper_layer::Layer>,
    layer_shown: bool,
    /// Where the wallpaper image goes on the screen, in physical pixels
    /// (`win::wallpaper::Picture`): left, top, width, height.
    wallpaper_place: (u32, u32, u32, u32),
    /// The appearance and size the wallpaper was decoded for, and whether a
    /// decode is under way.
    wallpaper_for: Option<(bool, u32, u32)>,
    wallpaper_loading: bool,
    /// The icons' places last reported to the CI trace.
    traced: Vec<(String, i32, i32)>,
    _subscriptions: Vec<Subscription>,
}

/// The desktop view, for the commands the bar and its menus run on it.
struct DesktopEntity(Entity<DesktopView>);

impl Global for DesktopEntity {}

impl DesktopView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let shell = shell(cx);
        let entity = cx.entity();
        cx.set_global(DesktopEntity(entity));
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let active = window.is_window_active();
            if this.active != active {
                this.active = active;
                trace(|| format!("desktop {}", if active { "active" } else { "inactive" }));
                if active {
                    // The desktop in front: the bar shows its menus, as the
                    // Mac's shows the Finder's.
                    this.shell.update(cx, |state, cx| {
                        state.front = None;
                        cx.notify();
                    });
                }
                cx.notify();
            }
        });
        let observe = cx.observe(&shell, |_, _, cx| cx.notify());
        Self {
            shell,
            focus: cx.focus_handle(),
            items: Vec::new(),
            settings: DesktopSettings::default(),
            selection: BTreeSet::new(),
            press: None,
            rename: None,
            active: false,
            wallpaper: None,
            layer: None,
            layer_shown: false,
            wallpaper_place: (0, 0, 0, 0),
            wallpaper_for: None,
            wallpaper_loading: false,
            traced: Vec::new(),
            _subscriptions: vec![activation, observe],
        }
    }

    fn grid(&self, window: &Window) -> Grid {
        let viewport = window.viewport_size();
        Grid::new(
            f32::from(viewport.width),
            f32::from(viewport.height),
            dock::DOCK_HEIGHT + 4.0,
            self.settings.view,
        )
    }

    /// Where every icon goes: saved places for an unsorted desktop (a new
    /// item takes the first free slot), or consecutive slots in the chosen
    /// order.
    fn layout(&self, grid: &Grid) -> Vec<Placed> {
        let placements: Vec<(usize, Placement)> = if self.settings.arrangement.is_sorted() {
            let mut order = (0..self.items.len()).collect::<Vec<_>>();
            let items = &self.items;
            match self.settings.arrangement {
                Arrangement::Kind => order.sort_by_key(|&index| {
                    (
                        !items[index].is_folder,
                        extension(&items[index].name),
                        items[index].name.to_lowercase(),
                    )
                }),
                Arrangement::DateModified => {
                    order.sort_by_key(|&index| std::cmp::Reverse(items[index].modified_millis))
                }
                Arrangement::Size => {
                    order.sort_by_key(|&index| std::cmp::Reverse(items[index].size_bytes))
                }
                _ => order.sort_by_key(|&index| items[index].name.to_lowercase()),
            }
            order
                .into_iter()
                .zip(grid.arrange(self.items.len()))
                .collect()
        } else {
            let keys = self.items.iter().map(key).collect::<Vec<_>>();
            grid.layout(keys.iter().map(String::as_str), &self.settings.positions)
                .into_iter()
                .enumerate()
                .collect()
        };
        placements
            .into_iter()
            .map(|(index, placement)| Placed {
                index,
                left: grid.left(placement),
                top: placement.top,
            })
            .collect()
    }

    /// The whole cell an icon and its label take, for hit-testing.
    fn cell(&self, grid: &Grid, placed: &Placed) -> Bounds<Pixels> {
        let options = grid.options;
        let pitch = options.pitch();
        Bounds {
            origin: point(
                px(placed.left + options.icon_size / 2.0 - pitch / 2.0),
                px(placed.top),
            ),
            size: size(
                px(pitch),
                px(options.icon_size + LABEL_GAP + options.label_line() * LABEL_LINES as f32),
            ),
        }
    }

    fn hit(&self, grid: &Grid, layout: &[Placed], position: Point<Pixels>) -> Option<usize> {
        layout
            .iter()
            .rev()
            .find(|placed| self.cell(grid, placed).contains(&position))
            .map(|placed| placed.index)
    }

    fn set_selection(&mut self, selection: BTreeSet<PathBuf>, cx: &mut Context<Self>) {
        if selection != self.selection {
            self.selection = selection;
            self.publish(cx);
            cx.notify();
        }
    }

    /// Tell the bar what its desktop menus act on.
    fn publish(&self, cx: &mut Context<Self>) {
        let count = self.selection.len();
        let only_files = self
            .items
            .iter()
            .filter(|item| self.selection.contains(&item.path))
            .all(|item| !item.is_folder);
        let sort = sort_choice(self.settings.arrangement);
        self.shell.update(cx, |state, cx| {
            if (
                state.desktop_selected,
                state.desktop_only_files,
                state.desktop_sort,
            ) != (count, only_files, sort)
            {
                state.desktop_selected = count;
                state.desktop_only_files = only_files;
                state.desktop_sort = sort;
                cx.notify();
            }
        });
    }

    fn selected_items(&self) -> Vec<DesktopItem> {
        self.items
            .iter()
            .filter(|item| self.selection.contains(&item.path))
            .cloned()
            .collect()
    }

    // ---- loading -------------------------------------------------------

    /// Read the Desktop folders again (off the UI thread).
    pub(crate) fn reload_items(&mut self, cx: &mut Context<Self>) {
        let listed = blocking::unblock(desktop_files::list);
        let settings_load = self.items.is_empty();
        let settings = settings_load.then(|| blocking::unblock(rmac_desktop::settings::load));
        cx.spawn(async move |this, cx| {
            let items = listed.await;
            let settings = match settings {
                Some(settings) => settings.await.ok(),
                None => None,
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(settings) = settings {
                    this.settings = settings;
                }
                trace(|| format!("desktop items {}", items.len()));
                this.items = items;
                let present = this
                    .items
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<BTreeSet<_>>();
                let selection = this
                    .selection
                    .intersection(&present)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                this.set_selection(selection, cx);
                this.publish(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Decode Lulo's wallpaper for this screen and appearance (off the UI
    /// thread), unless it already is.
    pub(crate) fn load_wallpaper(&mut self, force: bool, window: &Window, cx: &mut Context<Self>) {
        let scale = window.scale_factor();
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) * scale).round() as u32;
        let height = (f32::from(viewport.height) * scale).round() as u32;
        let dark = mac::is_dark();
        if width == 0 || height == 0 || self.wallpaper_loading {
            return;
        }
        // Not yet placed over the screen: the picture is decoded once, at
        // the screen's size, when it is.
        let (monitor, _) = surface::primary_monitor();
        let screen = (
            (monitor.right - monitor.left) as u32,
            (monitor.bottom - monitor.top) as u32,
        );
        if (width.abs_diff(screen.0) > 2) || (height.abs_diff(screen.1) > 2) {
            return;
        }
        if !force && self.wallpaper_for == Some((dark, width, height)) {
            return;
        }
        self.wallpaper_loading = true;
        let bar = (BAR_HEIGHT * scale).round() as u32;
        let dock_strip = (dock::DOCK_HEIGHT * scale).round() as u32;
        let loaded =
            blocking::unblock(move || wallpaper::load(width, height, bar, dock_strip, dark));
        cx.spawn_in(window, async move |this, cx| {
            let picture = loaded.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.wallpaper_loading = false;
                this.wallpaper_for = Some((dark, width, height));
                let Some(picture) = picture else {
                    return;
                };
                // Windows keeps the layer's pixels; the picture is freed
                // here. Only if the layer cannot show it does GPUI draw it.
                if this.layer.is_none() {
                    this.layer = wallpaper_layer::Layer::new();
                }
                let backdrop = mac::window().to_rgb();
                let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                let desktop = surface::hwnd(window);
                let (monitor, _) = surface::primary_monitor();
                this.layer_shown =
                    this.layer
                        .as_ref()
                        .zip(desktop)
                        .is_some_and(|(layer, desktop)| {
                            layer.show(
                                desktop,
                                monitor,
                                picture.left,
                                picture.top,
                                picture.width,
                                picture.height,
                                &picture.bgra,
                                [
                                    channel(backdrop.b),
                                    channel(backdrop.g),
                                    channel(backdrop.r),
                                ],
                            )
                        });
                let shown = this.layer_shown;
                trace(|| {
                    format!(
                        "desktop wallpaper drawn by {}",
                        if shown { "its layer" } else { "GPUI" }
                    )
                });
                let image = (!shown)
                    .then(|| {
                        image::RgbaImage::from_raw(picture.width, picture.height, picture.bgra)
                    })
                    .flatten()
                    .map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)])));
                if let Some(old) = std::mem::replace(&mut this.wallpaper, image) {
                    cx.drop_image(old, Some(window));
                }
                this.wallpaper_place = (picture.left, picture.top, picture.width, picture.height);
                let dark_text = wallpaper::dark_text_on(picture.bar_luminance);
                let strip_image = |strip: wallpaper::Strip| {
                    image::RgbaImage::from_raw(strip.width, strip.height, strip.bgra)
                        .map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)])))
                };
                let bar_backdrop = strip_image(picture.bar_strip);
                let dock_backdrop = strip_image(picture.dock_strip);
                this.shell.update(cx, |state, cx| {
                    state.bar_dark_text = Some(dark_text);
                    let old = [
                        std::mem::replace(&mut state.bar_backdrop, bar_backdrop),
                        std::mem::replace(&mut state.dock_backdrop, dock_backdrop),
                    ];
                    for image in old.into_iter().flatten() {
                        cx.drop_image(image, None);
                    }
                    cx.notify();
                });
                cx.notify();
            });
        })
        .detach();
    }

    // ---- commands ------------------------------------------------------

    fn open(&self, items: &[DesktopItem]) {
        for item in items {
            trace(|| format!("desktop open {}", item.name));
            let path = item.path.to_string_lossy().into_owned();
            launch::open(if item.is_folder {
                launch::folder_request(&path)
            } else {
                launch::Request::Shell(path)
            });
        }
    }

    fn recycle(&mut self, cx: &mut Context<Self>) {
        let paths = self.selection.iter().cloned().collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        let done = blocking::unblock(move || desktop_files::recycle(&paths));
        cx.spawn(async move |this, cx| {
            done.await;
            let _ = this.update(cx, |this, cx| this.reload_items(cx));
        })
        .detach();
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let made = blocking::unblock(desktop_files::new_folder);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = made.await else {
                return;
            };
            let listed = blocking::unblock(desktop_files::list).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.items = listed;
                this.set_selection(BTreeSet::from([path]), cx);
                this.begin_rename(window, cx);
            });
        })
        .detach();
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let paths = self
            .selected_items()
            .into_iter()
            .filter(|item| !item.is_folder)
            .map(|item| item.path)
            .collect::<Vec<_>>();
        let made = blocking::unblock(move || {
            paths
                .iter()
                .filter_map(|path| desktop_files::duplicate(path))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let made = made.await;
            let _ = this.update(cx, |this, cx| {
                this.set_selection(made.into_iter().collect(), cx);
                this.reload_items(cx);
            });
        })
        .detach();
    }

    fn make_shortcuts(&mut self, cx: &mut Context<Self>) {
        let paths = self.selection.iter().cloned().collect::<Vec<_>>();
        let made = blocking::unblock(move || {
            paths
                .iter()
                .filter_map(|path| desktop_files::make_shortcut(path))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let made = made.await;
            let _ = this.update(cx, |this, cx| {
                this.set_selection(made.into_iter().collect(), cx);
                this.reload_items(cx);
            });
        })
        .detach();
    }

    fn show_info(&self) {
        for item in self.selected_items() {
            let path = item.path.clone();
            std::thread::spawn(move || desktop_files::properties(&path));
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let all = self.items.iter().map(|item| item.path.clone()).collect();
        self.set_selection(all, cx);
    }

    fn set_arrangement(&mut self, arrangement: Arrangement, cx: &mut Context<Self>) {
        self.settings.arrangement = arrangement;
        self.save_settings();
        self.publish(cx);
        cx.notify();
    }

    /// Clean Up: every icon to the nearest free grid slot.
    fn clean_up(&mut self, window: &Window, cx: &mut Context<Self>) {
        let grid = self.grid(window);
        let layout = self.layout(&grid);
        let placements = layout
            .iter()
            .map(|placed| grid.placement_at(placed.left, placed.top))
            .collect::<Vec<_>>();
        let cleaned = grid.clean_up(&placements);
        for (placed, placement) in layout.iter().zip(cleaned) {
            self.settings
                .positions
                .insert(key(&self.items[placed.index]), placement);
        }
        self.settings.arrangement = Arrangement::None;
        self.save_settings();
        self.publish(cx);
        cx.notify();
    }

    fn save_settings(&self) {
        let settings = self.settings.clone();
        std::thread::spawn(move || {
            if let Err(error) = rmac_desktop::settings::save(&settings) {
                eprintln!("lulo-shell: the desktop's icon places were not saved: {error}");
            }
        });
    }

    // ---- renaming ------------------------------------------------------

    fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut selected = self.selected_items();
        if selected.len() != 1 {
            return;
        }
        let item = selected.remove(0);
        if item.shared {
            // The public Desktop needs an administrator; Lulo never asks.
            return;
        }
        let field = cx.new(|cx| InputState::new(window, cx));
        let name = item.name.clone();
        let stem = rmac_desktop::rename::editable_stem(&name, item.is_folder);
        field.update(cx, |field, cx| {
            field.set_value(name.clone(), window, cx);
            field.set_selected_range(stem, cx);
            field.focus(window, cx);
        });
        let subscription = cx.subscribe_in(
            &field,
            window,
            |this, field, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    let text = field.read(cx).value().to_string();
                    this.commit_rename(text, window, cx);
                }
                _ => {}
            },
        );
        trace(|| format!("desktop rename {}", item.name));
        self.rename = Some(Rename {
            item,
            field,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    fn commit_rename(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        self.focus.focus(window, cx);
        cx.notify();
        let text = text.trim().to_owned();
        if text.is_empty() || text == rename.item.name {
            return;
        }
        let item = rename.item.clone();
        let renamed = blocking::unblock(move || {
            desktop_files::rename(&item, &text).map_err(|error| (error, text))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = renamed.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(path) => {
                    trace(|| format!("desktop renamed to {}", path.display()));
                    // The icon keeps its place under its new name.
                    if let Some(placement) = this.settings.positions.remove(&key(&rename.item)) {
                        if let Some(name) = path.file_name() {
                            this.settings
                                .positions
                                .insert(name.to_string_lossy().into_owned(), placement);
                            this.save_settings();
                        }
                    }
                    this.set_selection(BTreeSet::from([path]), cx);
                    this.reload_items(cx);
                }
                Err((error, text)) => {
                    let (message, detail) = match error {
                        rmac_desktop::RenameError::Taken => (
                            rmac_desktop::rename::taken_message(&text),
                            "Please choose a different name.".to_owned(),
                        ),
                        rmac_desktop::RenameError::Invalid => {
                            let (message, detail) = rmac_desktop::rename::invalid_message(&text);
                            (message, detail.to_owned())
                        }
                        rmac_desktop::RenameError::Io(kind) => {
                            let (message, detail) =
                                rmac_desktop::rename::failed_message(&text, kind);
                            (message, detail.to_owned())
                        }
                    };
                    let answer = window.prompt(
                        gpui::PromptLevel::Warning,
                        &message,
                        Some(&detail),
                        &["OK"],
                        cx,
                    );
                    cx.spawn(async move |_, _| {
                        let _ = answer.await;
                    })
                    .detach();
                }
            });
        })
        .detach();
    }

    // ---- input ---------------------------------------------------------

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(rename) = &self.rename {
            let text = rename.field.read(cx).value().to_string();
            self.commit_rename(text, window, cx);
        }
        self.focus.focus(window, cx);
        let grid = self.grid(window);
        let layout = self.layout(&grid);
        let extend = event.modifiers.control || event.modifiers.shift || event.modifiers.platform;
        match self.hit(&grid, &layout, event.position) {
            Some(index) => {
                let path = self.items[index].path.clone();
                if event.click_count >= 2 && !extend {
                    let item = self.items[index].clone();
                    self.set_selection(BTreeSet::from([path]), cx);
                    self.open(&[item]);
                    return;
                }
                let mut selection = self.selection.clone();
                if extend {
                    if !selection.remove(&path) {
                        selection.insert(path);
                    }
                } else if !selection.contains(&path) {
                    selection = BTreeSet::from([path]);
                }
                self.set_selection(selection, cx);
                self.press = Some(Press {
                    start: event.position,
                    current: event.position,
                    kind: PressKind::Icons { dragging: false },
                });
            }
            None => {
                let base = if extend {
                    self.selection.clone()
                } else {
                    BTreeSet::new()
                };
                self.set_selection(base.clone(), cx);
                self.press = Some(Press {
                    start: event.position,
                    current: event.position,
                    kind: PressKind::Marquee { base },
                });
            }
        }
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.press.as_mut() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        press.current = event.position;
        let moved = press.current - press.start;
        match &mut press.kind {
            PressKind::Icons { dragging } => {
                if !*dragging
                    && f32::from(moved.x).abs().max(f32::from(moved.y).abs()) >= DRAG_THRESHOLD
                {
                    *dragging = true;
                }
                cx.notify();
            }
            PressKind::Marquee { base } => {
                let base = base.clone();
                let rect = marquee(press.start, press.current);
                let grid = self.grid(window);
                let layout = self.layout(&grid);
                let mut selection = base;
                for placed in &layout {
                    if self.cell(&grid, placed).intersects(&rect) {
                        selection.insert(self.items[placed.index].path.clone());
                    }
                }
                self.set_selection(selection, cx);
                cx.notify();
            }
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.press.take() else {
            return;
        };
        cx.notify();
        let PressKind::Icons { dragging: true } = press.kind else {
            return;
        };
        let grid = self.grid(window);
        let layout = self.layout(&grid);
        // Dropped on a folder that is not being dragged: the items move
        // into it.
        if let Some(target) = self.hit(&grid, &layout, event.position) {
            let folder = self.items[target].clone();
            if folder.is_folder && !self.selection.contains(&folder.path) {
                let items = self.selection.iter().cloned().collect::<Vec<_>>();
                let moved =
                    blocking::unblock(move || desktop_files::move_into(&items, &folder.path));
                cx.spawn(async move |this, cx| {
                    let count = moved.await;
                    trace(|| format!("desktop moved {count} item(s) into a folder"));
                    let _ = this.update(cx, |this, cx| this.reload_items(cx));
                })
                .detach();
                return;
            }
        }
        // Anywhere else: the dragged icons stay where they were dropped.
        let delta = event.position - press.start;
        for placed in &layout {
            let item = &self.items[placed.index];
            if !self.selection.contains(&item.path) {
                continue;
            }
            let placement = grid.clamp(grid.placement_at(
                placed.left + f32::from(delta.x),
                placed.top + f32::from(delta.y),
            ));
            self.settings.positions.insert(key(item), placement);
        }
        // Keep everything else where it is now that icons are placed by hand.
        if self.settings.arrangement.is_sorted() {
            for placed in &layout {
                let item = &self.items[placed.index];
                self.settings
                    .positions
                    .entry(key(item))
                    .or_insert(grid.placement_at(placed.left, placed.top));
            }
            self.settings.arrangement = Arrangement::None;
        }
        self.save_settings();
    }

    fn right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let grid = self.grid(window);
        let layout = self.layout(&grid);
        let items = match self.hit(&grid, &layout, event.position) {
            Some(index) => {
                let path = self.items[index].path.clone();
                if !self.selection.contains(&path) {
                    self.set_selection(BTreeSet::from([path]), cx);
                }
                let selected = self.selected_items();
                menus::desktop_item_menu(
                    selected.len(),
                    selected.iter().all(|item| !item.is_folder),
                    selected.iter().any(|item| item.shared),
                )
            }
            None => {
                self.set_selection(BTreeSet::new(), cx);
                menus::desktop_background_menu(sort_choice(self.settings.arrangement))
            }
        };
        let scale = window.scale_factor();
        let (monitor, _) = surface::primary_monitor();
        let at = (
            monitor.left + (f32::from(event.position.x) * scale).round() as i32,
            monitor.top + (f32::from(event.position.y) * scale).round() as i32,
        );
        cx.defer(move |cx| super::menu::open_dock_menu(items, at, cx));
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            if event.keystroke.key == "escape" {
                self.cancel_rename(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let modifiers = &event.keystroke.modifiers;
        let command = modifiers.control || modifiers.platform;
        match event.keystroke.key.as_str() {
            "enter" | "f2" => self.begin_rename(window, cx),
            "o" if command => self.open(&self.selected_items()),
            "delete" => self.recycle(cx),
            "backspace" if command => self.recycle(cx),
            "a" if command => self.select_all(cx),
            "escape" => self.set_selection(BTreeSet::new(), cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Run a desktop command chosen in the bar or a desktop menu.
    fn command(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            menus::DESKTOP_NEW_FOLDER => self.new_folder(window, cx),
            menus::DESKTOP_OPEN => self.open(&self.selected_items()),
            menus::DESKTOP_RECYCLE => self.recycle(cx),
            menus::DESKTOP_INFO => self.show_info(),
            menus::DESKTOP_RENAME => self.begin_rename(window, cx),
            menus::DESKTOP_DUPLICATE => self.duplicate(cx),
            menus::DESKTOP_SHORTCUT => self.make_shortcuts(cx),
            menus::DESKTOP_SELECT_ALL => self.select_all(cx),
            menus::DESKTOP_CLEAN_UP => self.clean_up(window, cx),
            menus::DESKTOP_SORT_NONE => self.set_arrangement(Arrangement::None, cx),
            menus::DESKTOP_SORT_NAME => self.set_arrangement(Arrangement::Name, cx),
            menus::DESKTOP_SORT_KIND => self.set_arrangement(Arrangement::Kind, cx),
            menus::DESKTOP_SORT_DATE => self.set_arrangement(Arrangement::DateModified, cx),
            menus::DESKTOP_SORT_SIZE => self.set_arrangement(Arrangement::Size, cx),
            _ => {}
        }
    }

    // ---- drawing -------------------------------------------------------

    /// The CI checks click icons by their place on screen: report each
    /// icon's centre (physical pixels) whenever the places change.
    fn trace_places(&mut self, layout: &[Placed], grid: &Grid, scale: f32) {
        let (monitor, _) = surface::primary_monitor();
        let icon = grid.options.icon_size;
        let places = layout
            .iter()
            .map(|placed| {
                (
                    self.items[placed.index].name.clone(),
                    monitor.left + ((placed.left + icon / 2.0) * scale).round() as i32,
                    monitor.top + ((placed.top + icon / 2.0) * scale).round() as i32,
                )
            })
            .collect::<Vec<_>>();
        if places != self.traced {
            for (name, x, y) in &places {
                trace(|| format!("desktop icon {name} at {x},{y}"));
            }
            self.traced = places;
        }
    }

    fn icon(
        &self,
        item: &DesktopItem,
        icon_size: f32,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if item.is_folder {
            return rmac_ui::svg_icon(IconSource::from(FOLDER_ICON), icon_size, scale, cx)
                .size(px(icon_size))
                .into_any_element();
        }
        let pixels = (icon_size * scale).round() as i32;
        let key = icons::desktop_key(&item.path, pixels);
        let cached = self.shell.read(cx).cached_icon(&key);
        match cached {
            Some(image) => img(image)
                .size(px(icon_size))
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => {
                let path = item.path.to_string_lossy().into_owned();
                self.shell
                    .update(cx, |state, _| state.want_desktop_icon(&key, &path, pixels));
                div().size(px(icon_size)).into_any_element()
            }
        }
    }

    fn tile(
        &self,
        placed: &Placed,
        offset: Point<Pixels>,
        grid: &Grid,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = grid.options;
        let icon_size = options.icon_size;
        let pitch = options.pitch();
        let item = &self.items[placed.index];
        let selected = self.selection.contains(&item.path);
        let icon = self.icon(item, icon_size, scale, cx);
        let editing = self
            .rename
            .as_ref()
            .filter(|rename| rename.item.path == item.path)
            .map(|rename| rename.field.clone());
        let mut shadow = mac::black();
        shadow.a = 0.65;
        let mut backdrop = mac::black();
        backdrop.a = 0.35;
        let mut inactive_pill = mac::white();
        inactive_pill.a = 0.25;
        let label: AnyElement = match editing {
            Some(field) => div()
                .mt(px(LABEL_GAP))
                .w(px(LABEL_MAX_WIDTH + 2.0 * LABEL_PAD))
                .rounded(px(LABEL_RADIUS))
                .bg(mac::window())
                .text_color(mac::text())
                .text_size(px(options.text_size))
                .child(TextField::new(&field).appearance(false))
                .into_any_element(),
            None => {
                let name = SharedString::from(item.name.clone());
                let text = |element: gpui::Div| {
                    element
                        .text_size(px(options.text_size))
                        .line_height(px(options.label_line()))
                        .text_center()
                        .whitespace_normal()
                        .line_clamp(LABEL_LINES)
                        .text_ellipsis()
                };
                div()
                    .mt(px(LABEL_GAP))
                    .max_w(px(LABEL_MAX_WIDTH))
                    .px(px(LABEL_PAD))
                    .rounded(px(LABEL_RADIUS))
                    .relative()
                    .when(selected, |label| {
                        label.bg(if self.active {
                            mac::accent()
                        } else {
                            inactive_pill
                        })
                    })
                    // White text over any wallpaper needs a shadow; a copy
                    // one point lower stands in for the Mac's blurred one.
                    // The copy sits exactly over the text's own box (inside
                    // the padding), so both wrap the same way.
                    .when(!selected, |label| {
                        label.child(
                            text(div())
                                .absolute()
                                .left(px(LABEL_PAD))
                                .right(px(LABEL_PAD))
                                .top(px(1.0))
                                .text_color(shadow)
                                .child(name.clone()),
                        )
                    })
                    .child(
                        text(div())
                            .relative()
                            .text_color(if selected && self.active {
                                mac::on_accent()
                            } else {
                                mac::white()
                            })
                            .child(name),
                    )
                    .into_any_element()
            }
        };
        let moving = selected
            && matches!(
                self.press,
                Some(Press {
                    kind: PressKind::Icons { dragging: true },
                    ..
                })
            );
        div()
            .id(("lulo-desktop-icon", placed.index))
            .role(Role::ListItem)
            .aria_label(SharedString::from(item.name.clone()))
            .aria_selected(selected)
            .absolute()
            .left(
                px(placed.left + icon_size / 2.0 - pitch / 2.0)
                    + if moving { offset.x } else { px(0.0) },
            )
            .top(px(placed.top) + if moving { offset.y } else { px(0.0) })
            .w(px(pitch))
            .flex()
            .flex_col()
            .items_center()
            .when(moving, |tile| tile.opacity(0.7))
            .child(
                div()
                    .relative()
                    .size(px(icon_size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |icon_box| {
                        icon_box.child(
                            div()
                                .absolute()
                                .left(px(-SELECTION_OUTSET))
                                .top(px(-SELECTION_OUTSET))
                                .size(px(icon_size + 2.0 * SELECTION_OUTSET))
                                .rounded(px(SELECTION_RADIUS))
                                .bg(backdrop),
                        )
                    })
                    .child(icon),
            )
            .child(label)
            .into_any_element()
    }
}

/// The Sort By row the menu ticks for `arrangement`.
fn sort_choice(arrangement: Arrangement) -> menus::DesktopSort {
    match arrangement {
        Arrangement::None | Arrangement::SnapToGrid => menus::DesktopSort::None,
        Arrangement::Name => menus::DesktopSort::Name,
        Arrangement::Kind => menus::DesktopSort::Kind,
        Arrangement::DateModified => menus::DesktopSort::DateModified,
        Arrangement::Size => menus::DesktopSort::Size,
    }
}

/// The key an item's saved place is kept under: its file name.
fn key(item: &DesktopItem) -> String {
    item.path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| item.name.clone())
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default()
}

fn marquee(start: Point<Pixels>, current: Point<Pixels>) -> Bounds<Pixels> {
    let origin = point(start.x.min(current.x), start.y.min(current.y));
    let end = point(start.x.max(current.x), start.y.max(current.y));
    Bounds {
        origin,
        size: size(end.x - origin.x, end.y - origin.y),
    }
}

impl Render for DesktopView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A new appearance or display size decodes the wallpaper again.
        self.load_wallpaper(false, window, cx);
        let scale = window.scale_factor();
        let grid = self.grid(window);
        let layout = self.layout(&grid);
        let offset = match &self.press {
            Some(press) => press.current - press.start,
            None => point(px(0.0), px(0.0)),
        };
        self.trace_places(&layout, &grid, scale);
        let tiles = layout
            .iter()
            .map(|placed| self.tile(placed, offset, &grid, scale, cx))
            .collect::<Vec<_>>();
        let marquee_rect = match &self.press {
            Some(Press {
                kind: PressKind::Marquee { .. },
                start,
                current,
            }) => Some(marquee(*start, *current)),
            _ => None,
        };
        let mut marquee_fill = mac::white();
        marquee_fill.a = 0.12;
        let mut marquee_edge = mac::white();
        marquee_edge.a = 0.5;
        div()
            .id("lulo-desktop")
            .role(Role::List)
            .aria_label("Desktop")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            // Until the wallpaper layer shows, the window's own colour.
            .when(!self.layer_shown, |desktop| desktop.bg(mac::window()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::right_mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_key_down(cx.listener(Self::key_down))
            .when_some(self.wallpaper.clone(), |desktop, image| {
                // Already the visible part at its drawn size: drawn 1:1.
                let (left, top, width, height) = self.wallpaper_place;
                let scale = window.scale_factor().max(0.5);
                desktop.child(
                    img(image)
                        .absolute()
                        .left(px(left as f32 / scale))
                        .top(px(top as f32 / scale))
                        .w(px(width as f32 / scale))
                        .h(px(height as f32 / scale))
                        .object_fit(ObjectFit::Fill),
                )
            })
            .children(tiles)
            .when_some(marquee_rect, |desktop, rect| {
                desktop.child(
                    div()
                        .absolute()
                        .left(rect.origin.x)
                        .top(rect.origin.y)
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .bg(marquee_fill)
                        .border_1()
                        .border_color(marquee_edge),
                )
            })
    }
}

// ---- the desktop window ----------------------------------------------------

/// Open Lulo mode's desktop window over the whole primary screen, just
/// above Explorer's desktop, and hide Explorer's desktop icons.
pub(crate) fn open(cx: &mut App) {
    let (monitor, _) = surface::primary_monitor();
    // The bar is open by now; its window knows the screen's scale, so the
    // desktop's first frame (and its wallpaper) is the screen's size.
    let scale = runtime(cx).bar.map_or(1.0, |bar| {
        surface::scale_factor(windows_list::handle(bar.hwnd))
    });
    let Some(desktop) = super::open_desktop_surface(super::logical(monitor, scale), cx) else {
        eprintln!("lulo-shell: the desktop could not open");
        return;
    };
    cx.global_mut::<super::Runtime>().desktop = Some(desktop);
    trace(|| format!("desktop window {}", desktop.hwnd));
    with_view(cx, |view, window, cx| {
        view.reload_items(cx);
        view.load_wallpaper(true, window, cx);
    });
    place(cx);
}

/// Fit the desktop window to the primary screen and put it back just
/// above Explorer's desktop.
pub(crate) fn place(cx: &mut App) {
    let Some(desktop) = runtime(cx).desktop else {
        return;
    };
    let (monitor, _) = surface::primary_monitor();
    later(cx, move || {
        let hwnd = windows_list::handle(desktop.hwnd);
        surface::show_in_place(hwnd, monitor);
        crate::win::desktop::place_above_desktop_layer(hwnd);
        crate::win::desktop::hide_icons();
        trace(|| {
            format!(
                "desktop at {},{},{},{}",
                monitor.left, monitor.top, monitor.right, monitor.bottom
            )
        });
    });
}

fn with_view(
    cx: &mut App,
    f: impl FnOnce(&mut DesktopView, &mut Window, &mut Context<DesktopView>),
) {
    let (Some(desktop), Some(entity)) = (
        runtime(cx).desktop,
        cx.try_global::<DesktopEntity>().map(|view| view.0.clone()),
    ) else {
        return;
    };
    let _ = desktop.handle.update(cx, |_, window, cx| {
        entity.update(cx, |view, cx| f(view, window, cx));
    });
}

/// The Desktop folders changed: read them again.
pub(crate) fn items_changed(cx: &mut App) {
    with_view(cx, |view, _, cx| view.reload_items(cx));
}

/// Lulo's wallpaper setting changed: decode it again.
pub(crate) fn wallpaper_changed(cx: &mut App) {
    with_view(cx, |view, window, cx| view.load_wallpaper(true, window, cx));
}

/// Run a desktop command from a menu. Commands that take the keyboard
/// (Rename) give the desktop window the foreground first.
pub(crate) fn command(action: &str, cx: &mut App) {
    if action == menus::DESKTOP_RENAME || action == menus::DESKTOP_NEW_FOLDER {
        if let Some(desktop) = runtime(cx).desktop {
            later(cx, move || windows_list::activate(desktop.hwnd));
        }
    }
    let action = action.to_owned();
    with_view(cx, |view, window, cx| view.command(&action, window, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marquee_is_the_rectangle_between_press_and_pointer() {
        let rect = marquee(point(px(50.0), px(40.0)), point(px(10.0), px(90.0)));
        assert_eq!(rect.origin, point(px(10.0), px(40.0)));
        assert_eq!(rect.size, size(px(40.0), px(50.0)));
    }

    #[test]
    fn extensions_are_compared_in_lower_case() {
        assert_eq!(extension("Report.PDF"), "pdf");
        assert_eq!(extension("folder"), "");
    }
}
