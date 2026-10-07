//! macOS-style right-click (context) menus.
//!
//! A macOS context menu is its own window: it can extend past the window it
//! was opened from and is kept on screen by the window server. On Wayland the
//! menu is therefore an `xdg_popup` ([`WindowKind::AnchoredPopup`]) anchored at
//! the click and constrained to the output with flip/slide, with an explicit
//! grab so it owns the keyboard and closes on a click in another app. Each
//! submenu is a child popup of its parent menu. Where the platform has no
//! popups (X11, macOS, tests) the same panel is drawn inside the window
//! instead.
//!
//! Metrics are the menu-bar dropdown's (design-lab/menus.html, measured on
//! macOS 26): 24 pt rows, 11 pt separators, 5 pt panel padding, 13 pt text,
//! the text column 16.5 pt in (24 with a checkmark column).

use gpui::{
    anchored, canvas, deferred, div, point, prelude::FluentBuilder as _, px, size, Action,
    AnyWindowHandle, App, AppContext as _, Bounds, ClickEvent, Context, ElementId, EntityId,
    FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, Pixels, Point, Render, Role, SharedString, Size,
    StatefulInteractiveElement as _, Styled as _, TextRun, Toggled, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};
use std::{
    cell::{Cell, RefCell},
    hash::{Hash, Hasher},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use crate::{
    components::{cycle_focus_within, DismissMenu, MENU_CONTEXT},
    mac,
    shortcuts::Shortcut,
};

// ---- Metrics (design-lab/menus.html) ---------------------------------------

const PANEL_PADDING: f32 = 5.0;
const ROW_INSET: f32 = 5.0;
const SEPARATOR_HEIGHT: f32 = 11.0;
const SEPARATOR_INSET: f32 = 16.0;
const TEXT_INSET: f32 = 16.5;
const CHECK_COLUMN: f32 = 7.5;
/// The checkmark is centred 14 pt from the panel edge.
const CHECK_CENTRE: f32 = 14.0;
const CHECK_BOX: f32 = 15.0;
const SWATCH_COLUMN: f32 = 18.0;
const SHORTCUT_GAP: f32 = 24.0;
const KEY_RIGHT: f32 = 14.0;
const CHEVRON_WIDTH: f32 = 5.0;
const CHEVRON_RIGHT: f32 = 17.0;
const MENU_TEXT: f32 = 13.0;
const HEADER_TEXT: f32 = 11.0;
/// Finder's tag row: seven 12 pt colour dots, each in an 18 pt target ringed
/// when applied or highlighted, 6 pt apart.
const TAG_ROW_HEIGHT: f32 = 26.0;
const TAG_DOT: f32 = 12.0;
const TAG_TARGET: f32 = 18.0;
const TAG_GAP: f32 = 6.0;
/// A submenu overlaps its parent by this much, as in the menu bar.
const SUBMENU_OVERLAP: f32 = 4.0;
/// Transparent room around the panel inside its pop-up window for the
/// shadow: the popover elevation's `shadow_margin()` (blur 18 + offset_y 6),
/// the reference pattern every other clipped-shadow surface in the product
/// now follows (`docs/parity.md` SESSION-07/-08, `crates/app-drawer`'s
/// `DRAWER_SHADOW_GUTTER`, `crates/rmac-launcher`'s `SHADOW_GUTTER`). Kept
/// as a constant rather than reading `theme::current()` here because the
/// popover elevation's blur/offset never change with contrast or appearance
/// (only its alpha does; see `rmac_design::elevation::Elevation::resolve`),
/// so the margin a surface must reserve is fixed — asserted against the
/// token in `shadow_margin_matches_the_popover_elevation_token` below.
const SHADOW_MARGIN: f32 = 24.0;
/// Type-select keeps extending the typed prefix within this pause.
const TYPE_SELECT_PAUSE: Duration = Duration::from_millis(1000);

fn row_height() -> f32 {
    rmac_design::Metrics::default().menu_row_height
}

fn min_width() -> f32 {
    rmac_design::Metrics::default().menu_min_width
}

// ---- Model -------------------------------------------------------------------

/// Checkmark column state for a menu row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuCheck {
    #[default]
    None,
    On,
    Mixed,
}

/// One colour dot in a [`ContextMenu::tag_row`].
pub struct MenuTag {
    pub label: SharedString,
    pub color: Hsla,
    pub checked: bool,
    pub action: Box<dyn Action>,
}

/// One entry in a [`ContextMenu`].
pub(crate) enum MenuEntry {
    Item {
        label: SharedString,
        shortcut: Option<SharedString>,
        action: Box<dyn Action>,
        danger: bool,
        enabled: bool,
        checked: MenuCheck,
        swatch: Option<Hsla>,
    },
    Separator,
    Header(SharedString),
    Submenu {
        label: SharedString,
        items: Rc<Vec<MenuEntry>>,
    },
    Tags {
        label: SharedString,
        tags: Vec<MenuTag>,
    },
}

impl MenuEntry {
    fn height(&self) -> f32 {
        match self {
            MenuEntry::Separator => SEPARATOR_HEIGHT,
            MenuEntry::Tags { .. } => TAG_ROW_HEIGHT,
            _ => row_height(),
        }
    }
}

/// First enabled item whose label starts with `query` (case-insensitive).
/// This is the pure model behind macOS type-select.
pub fn type_select_match(labels: &[(&str, bool)], query: &str) -> Option<usize> {
    let query = query.to_lowercase();
    labels
        .iter()
        .position(|(label, enabled)| *enabled && label.to_lowercase().starts_with(&query))
}

/// Column layout shared by every row of one panel.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Columns {
    checks: bool,
    swatches: bool,
}

impl Columns {
    fn of(entries: &[MenuEntry]) -> Self {
        let mut columns = Columns {
            checks: false,
            swatches: false,
        };
        for entry in entries {
            if let MenuEntry::Item {
                checked, swatch, ..
            } = entry
            {
                columns.checks |= *checked != MenuCheck::None;
                columns.swatches |= swatch.is_some();
            }
        }
        columns
    }

    /// Where row text starts, from the panel's left edge.
    fn text_x(self) -> f32 {
        TEXT_INSET
            + if self.checks { CHECK_COLUMN } else { 0.0 }
            + if self.swatches { SWATCH_COLUMN } else { 0.0 }
    }
}

/// Panel height for `entries`, padding included.
fn panel_height(entries: &[MenuEntry]) -> f32 {
    PANEL_PADDING * 2.0 + entries.iter().map(MenuEntry::height).sum::<f32>()
}

/// Top of entry `index` from the panel's top edge.
fn entry_top(entries: &[MenuEntry], index: usize) -> f32 {
    PANEL_PADDING
        + entries
            .iter()
            .take(index)
            .map(MenuEntry::height)
            .sum::<f32>()
}

/// Panel width: the widest row's content, as on macOS, never below the
/// minimum menu width. `text_width(text, size, weight)` measures one line.
fn panel_width(entries: &[MenuEntry], text_width: &dyn Fn(&str, f32, FontWeight) -> f32) -> f32 {
    let columns = Columns::of(entries);
    let text_x = columns.text_x();
    entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Item {
                label, shortcut, ..
            } => {
                let trailing = match shortcut {
                    Some(shortcut) => {
                        SHORTCUT_GAP
                            + text_width(shortcut, MENU_TEXT, FontWeight::NORMAL)
                            + KEY_RIGHT
                    }
                    None => TEXT_INSET,
                };
                text_x + text_width(label, MENU_TEXT, FontWeight::NORMAL) + trailing
            }
            MenuEntry::Submenu { label, .. } => {
                text_x
                    + text_width(label, MENU_TEXT, FontWeight::NORMAL)
                    + SHORTCUT_GAP
                    + CHEVRON_WIDTH
                    + CHEVRON_RIGHT
            }
            MenuEntry::Header(label) => {
                text_x + text_width(label, HEADER_TEXT, FontWeight::SEMIBOLD) + TEXT_INSET
            }
            MenuEntry::Tags { tags, .. } => {
                let count = tags.len() as f32;
                text_x + count * TAG_TARGET + (count - 1.0).max(0.0) * TAG_GAP + TEXT_INSET
            }
            MenuEntry::Separator => 0.0,
        })
        .fold(min_width(), f32::max)
        .ceil()
}

/// The panel size for `entries`, measured in `family`, the font the menu is
/// drawn in (a new pop-up window has no inherited text style of its own).
fn measure(entries: &[MenuEntry], family: &SharedString, window: &Window) -> Size<Pixels> {
    let style = window.text_style();
    let text_width = |text: &str, text_size: f32, weight: FontWeight| -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let mut font = style.font();
        font.family = family.clone();
        font.weight = weight;
        let run = TextRun {
            len: text.len(),
            font,
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            crate::text_px(text_size),
            &[run],
            None,
        );
        f32::from(line.width())
    };
    size(
        px(panel_width(entries, &text_width)),
        px(panel_height(entries)),
    )
}

/// A cheap fingerprint of what the menu shows, so an unchanged menu does not
/// redraw its pop-up every time the window behind it renders.
fn signature(entries: &[MenuEntry]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fn feed(entries: &[MenuEntry], hasher: &mut std::collections::hash_map::DefaultHasher) {
        for entry in entries {
            match entry {
                MenuEntry::Item {
                    label,
                    shortcut,
                    danger,
                    enabled,
                    checked,
                    swatch,
                    action,
                } => {
                    0u8.hash(hasher);
                    label.hash(hasher);
                    shortcut.hash(hasher);
                    (danger, enabled, checked).hash(hasher);
                    swatch.map(|c| format!("{c:?}")).hash(hasher);
                    action.name().hash(hasher);
                }
                MenuEntry::Separator => 1u8.hash(hasher),
                MenuEntry::Header(label) => {
                    2u8.hash(hasher);
                    label.hash(hasher);
                }
                MenuEntry::Submenu { label, items } => {
                    3u8.hash(hasher);
                    label.hash(hasher);
                    feed(items, hasher);
                }
                MenuEntry::Tags { label, tags } => {
                    4u8.hash(hasher);
                    label.hash(hasher);
                    for tag in tags {
                        tag.label.hash(hasher);
                        tag.checked.hash(hasher);
                    }
                }
            }
        }
    }
    feed(entries, &mut hasher);
    hasher.finish()
}

// ---- State -------------------------------------------------------------------

/// The pop-up window behind one open menu, shared by every clone of its
/// [`ContextMenuState`]. Dropping the last clone (the app dismissing the
/// menu) drops `_alive`, which closes the pop-up.
struct PopupLink {
    entries: RefCell<Rc<Vec<MenuEntry>>>,
    signature: Cell<u64>,
    window: Cell<Option<AnyWindowHandle>>,
    opening: Cell<bool>,
    /// The platform has no pop-up windows: draw the menu in the window.
    in_window: Cell<bool>,
    _alive: async_channel::Sender<()>,
    closed: async_channel::Receiver<()>,
}

thread_local! {
    /// macOS shows one context menu at a time.
    static OPEN_POPUP: Cell<Option<AnyWindowHandle>> = const { Cell::new(None) };
    /// The open menu pop-ups, innermost submenu last: where menu keys go.
    static KEY_TARGETS: RefCell<Vec<AnyWindowHandle>> = const { RefCell::new(Vec::new()) };
}

/// While a menu is open it takes every key, as on macOS. A compositor that
/// keeps the keyboard on the window under a grabbing pop-up (Sway does)
/// sends menu keys to that window instead; pass them on to the innermost
/// open menu before any of the window's own bindings see them.
pub(crate) fn init(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        let Some(target) = KEY_TARGETS.with(|targets| targets.borrow().last().copied()) else {
            return;
        };
        if target == window.window_handle() {
            return;
        }
        cx.stop_propagation();
        let keystroke = event.keystroke.clone();
        cx.defer(move |cx| {
            let _ = target.update(cx, |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
        });
    })
    .detach();
}

/// Focus and placement state for one open context menu.
///
/// The menu temporarily owns keyboard focus so its Escape binding is reliable,
/// while retaining the invoking control's focus for every dismissal path.
#[derive(Clone)]
pub struct ContextMenuState {
    position: Point<Pixels>,
    menu_focus: FocusHandle,
    return_focus: FocusHandle,
    active_submenu: Rc<Cell<Option<usize>>>,
    owner: EntityId,
    submenu_opens_left: bool,
    link: Rc<PopupLink>,
}

impl ContextMenuState {
    /// Open a menu at `position`, transfer keyboard focus to it, and remember
    /// the invoking control so focus can be restored when it closes.
    pub fn open<V: 'static>(
        position: Point<Pixels>,
        return_focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let menu_focus = cx.focus_handle();
        window.focus(&menu_focus, cx);
        let (alive, closed) = async_channel::bounded(1);
        Self {
            position,
            menu_focus,
            return_focus: return_focus.clone(),
            active_submenu: Rc::new(Cell::new(None)),
            owner: cx.entity_id(),
            // In-window menus only: the parent menu has a 180 px minimum
            // width; reserve the same space for its flyout and keep an 8 px
            // window margin.
            submenu_opens_left: position.x + px(380.0) > window.bounds().size.width - px(8.0),
            link: Rc::new(PopupLink {
                entries: RefCell::new(Rc::new(Vec::new())),
                signature: Cell::new(0),
                window: Cell::new(None),
                opening: Cell::new(false),
                in_window: Cell::new(false),
                _alive: alive,
                closed,
            }),
        }
    }

    /// The handle the open menu holds focus with; focus leaving it means
    /// the menu was chosen from or dismissed.
    pub(crate) fn menu_focus(&self) -> &FocusHandle {
        &self.menu_focus
    }

    /// Cursor-relative position used to anchor the menu.
    pub fn position(&self) -> Point<Pixels> {
        self.position
    }

    /// Close `menu`, returning focus to the control that opened it.
    pub fn dismiss(menu: &mut Option<Self>, window: &mut Window, cx: &mut App) -> bool {
        let Some(menu) = menu.take() else {
            return false;
        };
        window.focus(&menu.return_focus, cx);
        true
    }
}

// ---- Builder -----------------------------------------------------------------

/// A macOS-style right-click context menu. Open a [`ContextMenuState`] in a
/// right-mouse handler, store that state in the app, and render this menu as the
/// LAST child of the app root. Choosing an item dispatches its action and the
/// [`DismissMenu`] action in the app's window; clicking away or pressing Escape
/// dispatches [`DismissMenu`]. The app binds `DismissMenu` to dismiss its menu
/// state.
///
/// Items dispatch GPUI actions (the same `Box::new(MyAction)` pattern apps
/// already use), so the menu needs no app-view type to wire its clicks.
pub struct ContextMenu {
    pos: Point<Pixels>,
    items: Vec<MenuEntry>,
}

impl ContextMenu {
    /// Start a menu anchored at `pos` (window-relative, e.g. a right-click's
    /// `event.position`).
    pub fn new(pos: Point<Pixels>) -> Self {
        Self {
            pos,
            items: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_item(
        mut self,
        label: impl Into<SharedString>,
        shortcut: Option<SharedString>,
        action: Box<dyn Action>,
        danger: bool,
        enabled: bool,
        checked: MenuCheck,
        swatch: Option<Hsla>,
    ) -> Self {
        // Shown in this platform's form ("Ctrl+Shift+S" on Windows); the
        // stored text also drives the panel width.
        let shortcut = shortcut.map(|sc| match crate::shortcuts::display_hint(&sc) {
            std::borrow::Cow::Borrowed(_) => sc,
            std::borrow::Cow::Owned(hint) => SharedString::from(hint),
        });
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut,
            action,
            danger,
            enabled,
            checked,
            swatch,
        });
        self
    }

    /// Append a normal item that dispatches `action` when chosen.
    pub fn item(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.push_item(label, None, action, false, true, MenuCheck::None, None)
    }

    /// Append a normal item using the shared binding and menu hint.
    pub fn command_item(
        self,
        label: impl Into<SharedString>,
        shortcut: Shortcut,
        action: Box<dyn Action>,
    ) -> Self {
        self.push_item(
            label,
            Some(shortcut.hint.into()),
            action,
            false,
            true,
            MenuCheck::None,
            None,
        )
    }

    /// Append a destructive item (red label, e.g. Delete / Move to Trash).
    pub fn danger_item(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.push_item(label, None, action, true, true, MenuCheck::None, None)
    }

    /// Append a destructive item using the shared binding and menu hint.
    pub fn danger_command_item(
        self,
        label: impl Into<SharedString>,
        shortcut: Shortcut,
        action: Box<dyn Action>,
    ) -> Self {
        self.push_item(
            label,
            Some(shortcut.hint.into()),
            action,
            true,
            true,
            MenuCheck::None,
            None,
        )
    }

    /// Append a row with every attribute given, for menus built from an
    /// `rmac_app_menu` table (the in-window menu strip, and the Lulo
    /// layer's menu bar on Windows).
    pub fn entry(
        self,
        label: impl Into<SharedString>,
        shortcut: Option<SharedString>,
        action: Box<dyn Action>,
        enabled: bool,
        checked: MenuCheck,
    ) -> Self {
        self.push_item(label, shortcut, action, false, enabled, checked, None)
    }

    /// Append a thin divider.
    pub fn separator(mut self) -> Self {
        self.items.push(MenuEntry::Separator);
        self
    }

    /// Append a non-interactive section header.
    pub fn header(mut self, label: impl Into<SharedString>) -> Self {
        self.items.push(MenuEntry::Header(label.into()));
        self
    }

    /// Append a disabled item (dimmed and skipped by keyboard navigation).
    pub fn disabled_item(self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.push_item(label, None, action, false, false, MenuCheck::None, None)
    }

    /// Append an item with a checkmark (`On`) or dash (`Mixed`) in the check
    /// column, as used by View menus.
    pub fn checked_item(
        self,
        label: impl Into<SharedString>,
        checked: MenuCheck,
        action: Box<dyn Action>,
    ) -> Self {
        self.push_item(label, None, action, false, true, checked, None)
    }

    /// Append a checked item with a color swatch separate from its check mark.
    /// The item's accessible name remains `label`; the swatch is named as a
    /// child image so assistive technology can distinguish its color purpose.
    pub fn checked_item_with_swatch(
        self,
        label: impl Into<SharedString>,
        checked: MenuCheck,
        swatch: Hsla,
        action: Box<dyn Action>,
    ) -> Self {
        self.push_item(label, None, action, false, true, checked, Some(swatch))
    }

    /// Append Finder's row of colour-tag dots. `label` is the row's accessible
    /// name; each dot is a checkable menu item named after its tag.
    pub fn tag_row(mut self, label: impl Into<SharedString>, tags: Vec<MenuTag>) -> Self {
        self.items.push(MenuEntry::Tags {
            label: label.into(),
            tags,
        });
        self
    }

    /// Append a flyout menu. Choosing its row opens the flyout without
    /// dismissing the parent menu; child actions dismiss the whole menu.
    pub fn submenu(mut self, label: impl Into<SharedString>, submenu: ContextMenu) -> Self {
        self.items.push(MenuEntry::Submenu {
            label: label.into(),
            items: Rc::new(submenu.items),
        });
        self
    }

    /// Build the menu. Render this as the LAST child of the app root: it is
    /// a click-away catcher over the window, and the menu itself opens as a
    /// pop-up window anchored at the menu's position.
    pub fn render(self, state: &ContextMenuState) -> impl IntoElement {
        if state.link.in_window.get() {
            return render_in_window(self, state).into_any_element();
        }
        let link = state.link.clone();
        let return_focus = state.return_focus.clone();
        let owner = state.owner;
        let pos = self.pos;
        let entries = self.items;
        catcher(state)
            .child(
                canvas(
                    move |_, window, cx| {
                        sync_popup(link, entries, pos, return_focus, owner, window, cx)
                    },
                    |_, _, _, _| {},
                )
                .size_0(),
            )
            .into_any_element()
    }
}

/// The full-window click-away catcher behind a menu. It keeps the Escape
/// binding for the moments keyboard focus is still in this window.
fn catcher(state: &ContextMenuState) -> gpui::Stateful<gpui::Div> {
    div()
        .absolute()
        .inset_0()
        .id("rmac-menu-scrim")
        .track_focus(&state.menu_focus)
        .key_context(MENU_CONTEXT)
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.dispatch_action(Box::new(DismissMenu), cx);
        })
        .on_mouse_down(MouseButton::Right, |_, window, cx| {
            window.dispatch_action(Box::new(DismissMenu), cx);
        })
}

/// Open the pop-up the first time the menu renders, or refresh its rows.
fn sync_popup(
    link: Rc<PopupLink>,
    entries: Vec<MenuEntry>,
    pos: Point<Pixels>,
    return_focus: FocusHandle,
    owner: EntityId,
    window: &mut Window,
    cx: &mut App,
) {
    let fingerprint = signature(&entries);
    let changed = fingerprint != link.signature.get();
    let font_family = window.text_style().font_family;
    // Only a pop-up about to open needs its size here; an open one measures
    // its own rows when they change.
    let panel = (link.window.get().is_none() && !link.opening.get())
        .then(|| measure(&entries, &font_family, window));
    if changed {
        link.signature.set(fingerprint);
        *link.entries.borrow_mut() = Rc::new(entries);
    }
    if let Some(popup) = link.window.get() {
        if changed {
            cx.defer(move |cx| {
                let _ = popup.update(cx, |_, window, _| window.refresh());
            });
        }
        return;
    }
    let Some(panel) = panel else {
        return;
    };
    link.opening.set(true);
    let parent = window.window_handle();
    let weak = Rc::downgrade(&link);
    cx.defer(move |cx| {
        let Some(link) = weak.upgrade() else {
            return;
        };
        close_open_popup(cx);
        let anchor = Bounds::new(pos, size(px(1.0), px(1.0)));
        let options = gpui::popup::PopupOptions {
            parent,
            anchor_rect: anchor,
            anchor: gpui::popup::PopupAnchor::TopLeft,
            gravity: gpui::popup::PopupGravity::BottomRight,
            constraint_adjustment: gpui::popup::PopupConstraintAdjustment::FLIP_X
                | gpui::popup::PopupConstraintAdjustment::FLIP_Y
                | gpui::popup::PopupConstraintAdjustment::SLIDE_X
                | gpui::popup::PopupConstraintAdjustment::SLIDE_Y
                | gpui::popup::PopupConstraintAdjustment::RESIZE_Y,
            offset: point(px(-SHADOW_MARGIN), px(-SHADOW_MARGIN)),
            grab: true,
        };
        let source = Source::Root(Rc::downgrade(&link));
        let target = PopupTarget {
            parent,
            return_focus,
        };
        match open_popup(options, panel, source, target, font_family, 0, cx) {
            Some(handle) => {
                link.window.set(Some(handle));
                OPEN_POPUP.with(|open| open.set(Some(handle)));
                let closed = link.closed.clone();
                cx.spawn(async move |cx| {
                    // Resolves when the menu state (and its sender) is dropped.
                    let _ = closed.recv().await;
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                })
                .detach();
            }
            None => {
                link.in_window.set(true);
                cx.notify(owner);
            }
        }
    });
}

fn close_open_popup(cx: &mut App) {
    if let Some(handle) = OPEN_POPUP.with(|open| open.take()) {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

#[allow(clippy::too_many_arguments)]
fn open_popup(
    options: gpui::popup::PopupOptions,
    panel: Size<Pixels>,
    source: Source,
    target: PopupTarget,
    font_family: SharedString,
    depth: usize,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let margin = px(SHADOW_MARGIN);
    let window_size = size(panel.width + margin * 2.0, panel.height + margin * 2.0);
    let window_options = WindowOptions {
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::AnchoredPopup(options),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Point::default(),
            window_size,
        ))),
        window_background: WindowBackgroundAppearance::Transparent,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    cx.open_window(window_options, move |window, cx| {
        cx.new(|cx| MenuPopup::new(source, target, font_family, depth, panel, window, cx))
    })
    .ok()
    .map(Into::into)
}

// ---- Pop-up window -------------------------------------------------------------

enum Source {
    /// The top-level menu, whose rows follow the app's latest render.
    Root(Weak<PopupLink>),
    /// A submenu's rows.
    Child(Rc<Vec<MenuEntry>>),
}

/// Where a chosen item's action goes: the app window that opened the menu,
/// with focus back on the control that opened it.
#[derive(Clone)]
struct PopupTarget {
    parent: AnyWindowHandle,
    return_focus: FocusHandle,
}

impl PopupTarget {
    fn run(&self, action: Option<Box<dyn Action>>, cx: &mut App) {
        let return_focus = self.return_focus.clone();
        let _ = self.parent.update(cx, move |_, window, cx| {
            window.focus(&return_focus, cx);
            // Dismiss first so actions that deliberately move focus (for
            // example Finder's inline Rename editor) keep their new focus.
            window.dispatch_action(Box::new(DismissMenu), cx);
            if let Some(action) = action {
                window.dispatch_action(action, cx);
            }
        });
    }
}

struct MenuPopup {
    source: Source,
    target: PopupTarget,
    font_family: SharedString,
    depth: usize,
    focus: FocusHandle,
    panel: Size<Pixels>,
    /// The open submenu: its row and pop-up window.
    child: Option<(usize, AnyWindowHandle)>,
    /// A submenu whose pop-up is being opened.
    opening_child: Option<usize>,
    /// An entry type-select chose before its row was drawn.
    pending_focus: Option<usize>,
    /// The focus handles of this render's selectable rows, by entry index.
    rows: Vec<PanelRow>,
    typed: String,
    typed_at: Option<Instant>,
    /// Set once this menu itself told the app to dismiss it, so closing the
    /// pop-up does not dismiss (and move focus) a second time.
    dismissed: Rc<Cell<bool>>,
    /// Chooses a row's action: in the app's window, then closes this menu.
    activate: ActivateHandler,
}

impl MenuPopup {
    fn new(
        source: Source,
        target: PopupTarget,
        font_family: SharedString,
        depth: usize,
        panel: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let handle = window.window_handle();
        KEY_TARGETS.with(|targets| targets.borrow_mut().push(handle));
        cx.on_release(move |_, _| {
            KEY_TARGETS.with(|targets| targets.borrow_mut().retain(|open| *open != handle));
        })
        .detach();
        let dismissed = Rc::new(Cell::new(false));
        // A pop-up closed by the window server (a click in another app)
        // dismisses the app's menu state too.
        if let Source::Root(link) = &source {
            let link = link.clone();
            let target = target.clone();
            let dismissed = dismissed.clone();
            cx.on_release(move |_, cx| {
                if !dismissed.get() && link.upgrade().is_some() {
                    let target = target.clone();
                    cx.defer(move |cx| target.run(None, cx));
                }
            })
            .detach();
        }
        let activate: ActivateHandler = Rc::new({
            let target = target.clone();
            let dismissed = dismissed.clone();
            move |action, window, cx| {
                dismissed.set(true);
                target.run(Some(action), cx);
                // The app dropping its menu state closes the pop-up; take
                // this one down now so it does not linger for a frame.
                window.remove_window();
            }
        });
        Self {
            source,
            target,
            font_family,
            depth,
            focus,
            panel,
            child: None,
            opening_child: None,
            pending_focus: None,
            rows: Vec::new(),
            typed: String::new(),
            typed_at: None,
            activate,
            dismissed,
        }
    }

    fn entries(&self) -> Rc<Vec<MenuEntry>> {
        match &self.source {
            Source::Root(link) => link
                .upgrade()
                .map(|link| link.entries.borrow().clone())
                .unwrap_or_default(),
            Source::Child(items) => items.clone(),
        }
    }

    fn close_child(&mut self, cx: &mut App) {
        if let Some((_, handle)) = self.child.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn open_child(
        &mut self,
        index: usize,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A submenu closed from its own keyboard (Left, Escape) is gone.
        if let Some((_, handle)) = self.child {
            if handle.update(cx, |_, _, _| ()).is_err() {
                self.child = None;
            }
        }
        if self.opening_child == Some(index) {
            return;
        }
        if self.child.is_some_and(|(open, _)| open == index) {
            if keyboard {
                if let Some((_, handle)) = self.child {
                    let _ = handle.update(cx, |_, window, cx| window.focus_next(cx));
                }
            }
            return;
        }
        self.close_child(cx);
        let entries = self.entries();
        let Some(MenuEntry::Submenu { items, .. }) = entries.get(index) else {
            return;
        };
        let items = items.clone();
        let margin = px(SHADOW_MARGIN);
        let row = Bounds::new(
            point(margin, margin + px(entry_top(&entries, index))),
            size(self.panel.width, px(row_height())),
        );
        let options = gpui::popup::PopupOptions {
            parent: window.window_handle(),
            anchor_rect: row,
            anchor: gpui::popup::PopupAnchor::TopRight,
            gravity: gpui::popup::PopupGravity::BottomRight,
            constraint_adjustment: gpui::popup::PopupConstraintAdjustment::FLIP_X
                | gpui::popup::PopupConstraintAdjustment::FLIP_Y
                | gpui::popup::PopupConstraintAdjustment::SLIDE_X
                | gpui::popup::PopupConstraintAdjustment::SLIDE_Y
                | gpui::popup::PopupConstraintAdjustment::RESIZE_Y,
            // The submenu's first row lines up with the row that opened it,
            // its panel overlapping the parent's edge.
            offset: point(
                -(margin + px(SUBMENU_OVERLAP)),
                -(margin + px(PANEL_PADDING)),
            ),
            grab: true,
        };
        let panel = measure(&items, &self.font_family, window);
        let target = self.target.clone();
        let font_family = self.font_family.clone();
        let depth = self.depth + 1;
        let this = cx.entity().downgrade();
        self.opening_child = Some(index);
        // Opening a window from inside this one's event handler is deferred
        // until the handler returns.
        cx.defer(move |cx| {
            let opened = open_popup(
                options,
                panel,
                Source::Child(items),
                target,
                font_family,
                depth,
                cx,
            );
            let _ = this.update(cx, |this, _| this.opening_child = None);
            let Some(handle) = opened else {
                return;
            };
            if keyboard {
                let _ = handle.update(cx, |_, window, cx| window.focus_next(cx));
            }
            let _ = this.update(cx, |this, cx| {
                this.child = Some((index, handle));
                cx.notify();
            });
        });
    }

    /// Close this submenu and return the keyboard to its parent menu.
    fn close_self(&mut self, window: &mut Window) {
        if self.depth > 0 {
            window.remove_window();
        }
    }

    fn type_select(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self
            .typed_at
            .is_none_or(|at| now.duration_since(at) > TYPE_SELECT_PAUSE)
        {
            self.typed.clear();
        }
        self.typed_at = Some(now);
        self.typed.push_str(key);
        let entries = self.entries();
        let labels: Vec<(&str, bool)> = entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item { label, enabled, .. } => (label.as_ref(), *enabled),
                MenuEntry::Submenu { label, .. } => (label.as_ref(), true),
                _ => ("", false),
            })
            .collect();
        if let Some(found) = type_select_match(&labels, &self.typed) {
            self.focus_entry(found, window, cx);
        }
    }

    /// Highlight entry `index`, or as soon as its row is drawn: keys can
    /// arrive before a new pop-up's first frame.
    fn focus_entry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.iter().find(|row| row.index == index) {
            Some(row) => window.focus(&row.handle, cx),
            None => {
                self.pending_focus = Some(index);
                cx.notify();
            }
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let focused = self.rows.iter().find(|row| row.handle.is_focused(window));
        let focused_row = focused.map(|row| row.index);
        let focused_action = focused.and_then(|row| row.action.as_ref().map(|a| a.boxed_clone()));
        let entries = self.entries();
        match key {
            // Return and Space choose the highlighted row here rather than
            // through the row's own keyboard click, which needs the key's
            // release too: keys a window passes on (see `init`) have none.
            "enter" | "space" if !keystroke.modifiers.modified() => {
                let Some(index) = focused_row else {
                    return;
                };
                cx.stop_propagation();
                match focused_action {
                    Some(action) => {
                        let activate = self.activate.clone();
                        activate(action, window, cx);
                    }
                    None => self.open_child(index, true, window, cx),
                }
            }
            "down" | "up" | "tab" => {
                cx.stop_propagation();
                let forward = match key {
                    "down" => true,
                    "up" => false,
                    _ => !keystroke.modifiers.shift,
                };
                cycle_focus_within(&self.focus, forward, window, cx);
            }
            "right" => {
                if let Some(index) = focused_row
                    .filter(|i| matches!(entries.get(*i), Some(MenuEntry::Submenu { .. })))
                {
                    cx.stop_propagation();
                    self.open_child(index, true, window, cx);
                }
            }
            "left" if self.depth > 0 => {
                cx.stop_propagation();
                self.close_self(window);
            }
            _ => {
                let modifiers = keystroke.modifiers;
                let plain = !modifiers.control && !modifiers.alt && !modifiers.platform;
                if plain && key.chars().count() == 1 && key != " " {
                    cx.stop_propagation();
                    self.type_select(key, window, cx);
                }
            }
        }
    }
}

impl Focusable for MenuPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MenuPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries();
        // Rows can change while the menu is open (a check mark, a label).
        let measured = measure(&entries, &self.font_family, window);
        if measured != self.panel {
            self.panel = measured;
            let margin = px(SHADOW_MARGIN);
            window.resize(size(
                measured.width + margin * 2.0,
                measured.height + margin * 2.0,
            ));
        }
        let margin = px(SHADOW_MARGIN);
        // Only the panel takes input; its shadow margin passes clicks to
        // whatever is underneath, as on macOS.
        window.set_input_region(Some(&[Bounds::new(point(margin, margin), self.panel)]));

        let this = cx.entity().downgrade();
        let open_child = self.child.map(|(index, _)| index);
        let ctx = PanelContext {
            depth: self.depth,
            open_submenu: open_child,
            on_submenu: Rc::new({
                let this = this.clone();
                move |index, keyboard, window, cx| {
                    let _ =
                        this.update(cx, |this, cx| this.open_child(index, keyboard, window, cx));
                }
            }),
            on_hover_item: Rc::new({
                let this = this.clone();
                move |_, cx| {
                    let _ = this.update(cx, |this, cx| {
                        if this.child.is_some() {
                            this.close_child(cx);
                            cx.notify();
                        }
                    });
                }
            }),
            activate: self.activate.clone(),
        };
        let (panel, rows) = render_panel(&entries, self.panel.width, &ctx, window, cx);
        self.rows = rows;
        if let Some(index) = self.pending_focus.take() {
            if let Some(row) = self.rows.iter().find(|row| row.index == index) {
                window.focus(&row.handle, cx);
            }
        }

        let target = self.target.clone();
        let dismissed = self.dismissed.clone();
        let depth = self.depth;
        div()
            .id("rmac-menu-popup")
            .size_full()
            .p(margin)
            .font_family(self.font_family.clone())
            .track_focus(&self.focus)
            .key_context(MENU_CONTEXT)
            .on_action(cx.listener(move |this, _: &DismissMenu, window, cx| {
                if depth > 0 {
                    // Escape in a submenu closes just that submenu.
                    this.close_self(window);
                } else {
                    dismissed.set(true);
                    target.run(None, cx);
                    window.remove_window();
                }
            }))
            .capture_key_down(cx.listener(Self::key_down))
            .child(panel)
    }
}

// ---- Panel ---------------------------------------------------------------------

/// A selectable row of a drawn panel: its entry, focus, and the action it
/// chooses (`None` for a submenu row).
struct PanelRow {
    index: usize,
    handle: FocusHandle,
    action: Option<Box<dyn Action>>,
}

type SubmenuHandler = Rc<dyn Fn(usize, bool, &mut Window, &mut App)>;
type HoverHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type ActivateHandler = Rc<dyn Fn(Box<dyn Action>, &mut Window, &mut App)>;

/// What a panel's rows do, which differs between a pop-up and the in-window
/// fallback.
struct PanelContext {
    depth: usize,
    open_submenu: Option<usize>,
    on_submenu: SubmenuHandler,
    on_hover_item: HoverHandler,
    activate: ActivateHandler,
}

fn row_id(depth: usize, kind: &'static str, index: usize) -> ElementId {
    ElementId::Name(format!("rmac-menu-{kind}-{depth}-{index}").into())
}

/// One menu panel. Returns it with the focus handles of its selectable rows.
fn render_panel(
    entries: &[MenuEntry],
    width: Pixels,
    ctx: &PanelContext,
    window: &mut Window,
    cx: &mut App,
) -> (gpui::Stateful<gpui::Div>, Vec<PanelRow>) {
    let columns = Columns::of(entries);
    let text_x = columns.text_x();
    let depth = ctx.depth;
    let mut rows = Vec::new();
    let mut panel = div()
        .id(ElementId::Name(format!("rmac-context-menu-{depth}").into()))
        .role(Role::Menu)
        .w(width)
        .py(px(PANEL_PADDING))
        .flex()
        .flex_col()
        .tab_group()
        .text_size(crate::text_px(MENU_TEXT))
        .text_color(mac::text())
        .rounded(px(mac::radius_menu()))
        .bg(mac::menu_surface())
        .border_1()
        .border_color(mac::separator())
        .shadow(mac::menu_shadow())
        .occlude();

    for (index, entry) in entries.iter().enumerate() {
        match entry {
            MenuEntry::Separator => {
                panel = panel.child(
                    div()
                        .id(row_id(depth, "separator", index))
                        .role(Role::Splitter)
                        .h(px(SEPARATOR_HEIGHT))
                        .flex()
                        .items_center()
                        .px(px(SEPARATOR_INSET))
                        .child(div().w_full().h(px(1.0)).bg(mac::separator())),
                );
            }
            MenuEntry::Header(label) => {
                panel = panel.child(
                    div()
                        .h(px(row_height()))
                        .pl(px(text_x))
                        .flex()
                        .items_center()
                        .text_size(crate::text_px(HEADER_TEXT))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(mac::text_secondary())
                        .child(label.clone()),
                );
            }
            MenuEntry::Tags { label, tags } => {
                let mut dots = div()
                    .id(row_id(depth, "tags", index))
                    .role(Role::MenuItem)
                    .aria_label(label.clone())
                    .h(px(TAG_ROW_HEIGHT))
                    .pl(px(text_x))
                    .flex()
                    .items_center()
                    .gap(px(TAG_GAP));
                for (dot, tag) in tags.iter().enumerate() {
                    let id = ElementId::Name(format!("rmac-menu-tag-{depth}-{index}-{dot}").into());
                    let handle = window
                        .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
                        .read(cx)
                        .clone();
                    let highlighted = handle.is_focused(window);
                    rows.push(PanelRow {
                        index,
                        handle: handle.clone(),
                        action: Some(tag.action.boxed_clone()),
                    });
                    let action = tag.action.boxed_clone();
                    let activate = ctx.activate.clone();
                    let hover = ctx.on_hover_item.clone();
                    let hover_handle = handle.clone();
                    dots = dots.child(
                        div()
                            .id(id)
                            .role(Role::MenuItemCheckBox)
                            .aria_label(tag.label.clone())
                            .aria_toggled(if tag.checked {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .track_focus(&handle.tab_stop(true))
                            .size(px(TAG_TARGET))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            // Applied tags and the hovered or focused dot get
                            // a ring, so the colour itself stays visible.
                            .border_2()
                            .border_color(if tag.checked || highlighted {
                                mac::text_secondary()
                            } else {
                                gpui::transparent_black()
                            })
                            .on_mouse_move(move |_, window, cx| {
                                if !hover_handle.is_focused(window) {
                                    window.focus(&hover_handle, cx);
                                    hover(window, cx);
                                }
                            })
                            .on_click(move |_, window, cx| {
                                activate(action.boxed_clone(), window, cx);
                            })
                            .child(
                                div()
                                    .id(ElementId::Name(
                                        format!("rmac-menu-tag-swatch-{depth}-{index}-{dot}")
                                            .into(),
                                    ))
                                    .role(Role::Image)
                                    .aria_label(format!("{} tag color", tag.label))
                                    .size(px(TAG_DOT))
                                    .rounded_full()
                                    .bg(tag.color),
                            ),
                    );
                }
                panel = panel.child(dots);
            }
            MenuEntry::Submenu { label, .. } => {
                let id = row_id(depth, "submenu", index);
                let handle = window
                    .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone();
                rows.push(PanelRow {
                    index,
                    handle: handle.clone(),
                    action: None,
                });
                let open = ctx.open_submenu == Some(index);
                let highlighted = open || handle.is_focused(window);
                let hover_handle = handle.clone();
                let on_hover = ctx.on_submenu.clone();
                let on_click = ctx.on_submenu.clone();
                panel = panel.child(
                    menu_row(id, highlighted)
                        .role(Role::MenuItem)
                        .aria_label(label.clone())
                        .aria_expanded(open)
                        .track_focus(&handle.tab_stop(true))
                        .on_mouse_move(move |_, window, cx| {
                            if !hover_handle.is_focused(window) {
                                window.focus(&hover_handle, cx);
                                on_hover(index, false, window, cx);
                            }
                        })
                        .on_click(move |event: &ClickEvent, window, cx| {
                            on_click(index, event.is_keyboard(), window, cx);
                        })
                        .child(leading(
                            columns,
                            MenuCheck::None,
                            None,
                            highlighted,
                            depth,
                            index,
                        ))
                        .child(div().flex_1().child(label.clone()))
                        .child(
                            div()
                                .pl(px(SHORTCUT_GAP))
                                .pr(px(CHEVRON_RIGHT - ROW_INSET - CHEVRON_WIDTH))
                                .text_color(if highlighted {
                                    mac::on_accent()
                                } else {
                                    mac::text_secondary()
                                })
                                .child("›"),
                        ),
                );
            }
            MenuEntry::Item {
                label,
                shortcut,
                action,
                danger,
                enabled,
                checked,
                swatch,
            } => {
                let id = row_id(depth, "item", index);
                let handle = window
                    .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone();
                let enabled = *enabled;
                let highlighted = enabled && handle.is_focused(window);
                if enabled {
                    rows.push(PanelRow {
                        index,
                        handle: handle.clone(),
                        action: Some(action.boxed_clone()),
                    });
                }
                let color = if !enabled {
                    mac::text_tertiary()
                } else if highlighted {
                    mac::on_accent()
                } else if *danger {
                    mac::danger()
                } else {
                    mac::text()
                };
                let toggled = match checked {
                    MenuCheck::On => Some(Toggled::True),
                    MenuCheck::Mixed => Some(Toggled::Mixed),
                    MenuCheck::None => None,
                };
                let role = if toggled.is_some() {
                    Role::MenuItemCheckBox
                } else {
                    Role::MenuItem
                };
                let hover_handle = handle.clone();
                let hover = ctx.on_hover_item.clone();
                let activate = ctx.activate.clone();
                let action = action.boxed_clone();
                // The visible row also carries the checkmark glyph and a
                // shortcut hint as sibling text runs, so a content-derived
                // name would read them out too. Every item gets an explicit
                // name of just its label instead.
                panel = panel.child(
                    menu_row(id, highlighted)
                        .role(role)
                        .aria_label(label.clone())
                        .when_some(toggled, |row, toggled| row.aria_toggled(toggled))
                        .text_color(color)
                        .when(enabled, |row| {
                            row.track_focus(&handle.tab_stop(true)).on_click(
                                move |_, window, cx| {
                                    activate(action.boxed_clone(), window, cx);
                                },
                            )
                        })
                        .on_mouse_move(move |_, window, cx| {
                            if enabled && !hover_handle.is_focused(window) {
                                window.focus(&hover_handle, cx);
                            }
                            hover(window, cx);
                        })
                        .child(leading(
                            columns,
                            *checked,
                            *swatch,
                            highlighted,
                            depth,
                            index,
                        ))
                        .child(div().flex_1().whitespace_nowrap().child(label.clone()))
                        .child(match shortcut {
                            Some(shortcut) => div()
                                .pl(px(SHORTCUT_GAP))
                                .pr(px(KEY_RIGHT - ROW_INSET))
                                .whitespace_nowrap()
                                .text_color(if highlighted {
                                    mac::on_accent()
                                } else {
                                    mac::text_tertiary()
                                })
                                .child(shortcut.clone()),
                            None => div().pr(px(TEXT_INSET - ROW_INSET)),
                        }),
                );
            }
        }
    }
    (panel, rows)
}

/// A selectable row's box: inset 5 pt from the panel edges, rounded, and
/// filled with the accent colour while highlighted.
fn menu_row(id: ElementId, highlighted: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(row_height()))
        .mx(px(ROW_INSET))
        .flex()
        .items_center()
        .rounded(px(mac::radius_menu_item()))
        .cursor_default()
        .when(highlighted, |row| {
            row.bg(mac::accent()).text_color(mac::on_accent())
        })
}

/// The check (and swatch) columns before a row's text: the checkmark centres
/// 14 pt from the panel edge, a tag swatch sits just before the text.
fn leading(
    columns: Columns,
    checked: MenuCheck,
    swatch: Option<Hsla>,
    highlighted: bool,
    depth: usize,
    index: usize,
) -> impl IntoElement {
    let mark = match checked {
        MenuCheck::On => "✓",
        MenuCheck::Mixed => "–",
        MenuCheck::None => "",
    };
    div()
        .flex_none()
        .w(px(columns.text_x() - ROW_INSET))
        .h_full()
        .flex()
        .items_center()
        .when(columns.checks, |el| {
            el.child(
                div()
                    .flex_none()
                    .ml(px(CHECK_CENTRE - ROW_INSET - CHECK_BOX / 2.0))
                    .w(px(CHECK_BOX))
                    .flex()
                    .justify_center()
                    .text_size(crate::text_px(12.0))
                    .child(mark),
            )
        })
        .child(div().flex_1())
        .when(columns.swatches, |el| {
            el.child(
                div()
                    .flex_none()
                    .w(px(SWATCH_COLUMN))
                    .flex()
                    .items_center()
                    .when_some(swatch, |el, color| {
                        el.child(
                            div()
                                .id(ElementId::Name(
                                    format!("rmac-context-menu-swatch-{depth}-{index}").into(),
                                ))
                                .role(Role::Image)
                                .size(px(9.0))
                                .rounded_full()
                                .bg(color)
                                .when(highlighted, |dot| {
                                    dot.border_1().border_color(mac::on_accent())
                                }),
                        )
                    }),
            )
        })
}

// ---- In-window fallback --------------------------------------------------------

/// The menu drawn inside the window, for platforms without pop-up windows.
fn render_in_window(menu: ContextMenu, state: &ContextMenuState) -> impl IntoElement {
    let pos = menu.pos;
    let entries = Rc::new(menu.items);
    let active_submenu = state.active_submenu.clone();
    let owner = state.owner;
    let return_focus = state.return_focus.clone();
    let opens_left = state.submenu_opens_left;
    let navigation_focus = state.menu_focus.clone();
    let escape_submenu = active_submenu.clone();
    catcher(state)
        .capture_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key.as_str() == "escape" && escape_submenu.get().is_some() {
                cx.stop_propagation();
                escape_submenu.set(None);
                cx.notify(owner);
                return;
            }
            let forward = match event.keystroke.key.as_str() {
                "down" => Some(true),
                "up" => Some(false),
                "tab" => Some(!event.keystroke.modifiers.shift),
                _ => None,
            };
            if let Some(forward) = forward {
                cx.stop_propagation();
                cycle_focus_within(&navigation_focus, forward, window, cx);
            }
        })
        .child(
            deferred(
                anchored()
                    .position(pos)
                    .snap_to_window_with_margin(px(8.0))
                    .child(InWindowMenu {
                        entries,
                        active_submenu,
                        owner,
                        return_focus,
                        opens_left,
                    }),
            )
            .with_priority(1),
        )
}

#[derive(IntoElement)]
struct InWindowMenu {
    entries: Rc<Vec<MenuEntry>>,
    active_submenu: Rc<Cell<Option<usize>>>,
    owner: EntityId,
    return_focus: FocusHandle,
    opens_left: bool,
}

impl gpui::RenderOnce for InWindowMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let activate: ActivateHandler = Rc::new({
            let return_focus = self.return_focus.clone();
            move |action, window, cx| {
                window.focus(&return_focus, cx);
                window.dispatch_action(Box::new(DismissMenu), cx);
                window.dispatch_action(action, cx);
            }
        });
        let set_submenu = |active: Rc<Cell<Option<usize>>>, owner: EntityId| -> SubmenuHandler {
            Rc::new(move |index, _, _, cx| {
                if active.get() != Some(index) {
                    active.set(Some(index));
                    cx.notify(owner);
                }
            })
        };
        let clear_submenu: HoverHandler = Rc::new({
            let active = self.active_submenu.clone();
            let owner = self.owner;
            move |_, cx| {
                if active.take().is_some() {
                    cx.notify(owner);
                }
            }
        });
        let open = self.active_submenu.get();
        let ctx = PanelContext {
            depth: 0,
            open_submenu: open,
            on_submenu: set_submenu(self.active_submenu.clone(), self.owner),
            on_hover_item: clear_submenu,
            activate: activate.clone(),
        };
        let family = window.text_style().font_family;
        let width = measure(&self.entries, &family, window).width;
        let (panel, _) = render_panel(&self.entries, width, &ctx, window, cx);
        let flyout = open.and_then(|index| match self.entries.get(index) {
            Some(MenuEntry::Submenu { items, .. }) => {
                let sub_ctx = PanelContext {
                    depth: 1,
                    open_submenu: None,
                    on_submenu: Rc::new(|_, _, _, _| {}),
                    on_hover_item: Rc::new(|_, _| {}),
                    activate: activate.clone(),
                };
                let sub_width = measure(items, &family, window).width;
                let (sub, _) = render_panel(items, sub_width, &sub_ctx, window, cx);
                let top = px(entry_top(&self.entries, index) - PANEL_PADDING);
                let sub = sub.absolute().top(top);
                Some(if self.opens_left {
                    sub.right(width - px(SUBMENU_OVERLAP))
                } else {
                    sub.left(width - px(SUBMENU_OVERLAP))
                })
            }
            _ => None,
        });
        div()
            .relative()
            .child(panel)
            .when_some(flyout, |el, flyout| el.child(flyout))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_select_is_case_insensitive_and_skips_disabled_items() {
        let labels = [
            ("Open", true),
            ("Open With", true),
            ("Duplicate", false),
            ("Delete", true),
        ];
        assert_eq!(type_select_match(&labels, "o"), Some(0));
        assert_eq!(type_select_match(&labels, "op"), Some(0));
        // "Duplicate" is disabled, so type-select lands on "Delete".
        assert_eq!(type_select_match(&labels, "d"), Some(3));
        assert_eq!(type_select_match(&labels, "de"), Some(3));
        assert_eq!(type_select_match(&labels, "z"), None);
        assert_eq!(type_select_match(&[], "a"), None);
    }

    #[test]
    fn menu_check_defaults_to_none() {
        assert_eq!(MenuCheck::default(), MenuCheck::None);
    }

    /// `SHADOW_MARGIN` is a constant approximation of the popover elevation
    /// token's `shadow_margin()` (its blur/offset never change with
    /// contrast, see `rmac_design::elevation::Elevation::resolve`) so every
    /// other surface following this reference pattern can read the same
    /// value without an `App` context. If the popover token ever moves,
    /// this fails loudly instead of leaving the shadow clipped again.
    #[test]
    fn shadow_margin_matches_the_popover_elevation_token() {
        let elevation = rmac_design::Elevation::resolve(rmac_appearance::Contrast::Normal);
        assert_eq!(SHADOW_MARGIN, elevation.popover.shadow_margin());
    }

    #[test]
    fn submenu_retains_its_checked_child_entry() {
        let pos = Point::new(px(0.0), px(0.0));
        let menu = ContextMenu::new(pos).submenu(
            "Sort By",
            ContextMenu::new(pos).checked_item("Name", MenuCheck::On, Box::new(DismissMenu)),
        );
        let [MenuEntry::Submenu { label, items }] = menu.items.as_slice() else {
            panic!("submenu should remain a nested menu entry");
        };
        assert_eq!(label.as_ref(), "Sort By");
        assert!(
            matches!(items.as_slice(), [MenuEntry::Item { label, checked: MenuCheck::On, .. }] if label.as_ref() == "Name")
        );
    }

    #[test]
    fn checked_swatch_item_keeps_its_label_and_check_state() {
        let pos = Point::new(px(0.0), px(0.0));
        let color: Hsla = gpui::rgb(0xff3b30).into();
        let menu = ContextMenu::new(pos).checked_item_with_swatch(
            "Red",
            MenuCheck::Mixed,
            color,
            Box::new(DismissMenu),
        );
        let [MenuEntry::Item {
            label,
            checked,
            swatch,
            ..
        }] = menu.items.as_slice()
        else {
            panic!("swatch should stay attached to one checked menu item");
        };
        assert_eq!(label.as_ref(), "Red");
        assert_eq!(*checked, MenuCheck::Mixed);
        assert_eq!(*swatch, Some(color));
    }

    fn fixed_width(text: &str, size: f32, _: FontWeight) -> f32 {
        // 7 px per character at 13 pt: deterministic for the layout tests.
        text.chars().count() as f32 * 7.0 * size / MENU_TEXT
    }

    #[test]
    fn panel_metrics_follow_the_measured_mac_menu() {
        let pos = Point::new(px(0.0), px(0.0));
        let menu = ContextMenu::new(pos)
            .item("Open", Box::new(DismissMenu))
            .separator()
            .item("Get Info", Box::new(DismissMenu));
        // 5 + 24 + 11 + 24 + 5.
        assert_eq!(panel_height(&menu.items), 69.0);
        assert_eq!(entry_top(&menu.items, 2), 40.0);
        // Short labels fall back to the minimum menu width.
        assert_eq!(panel_width(&menu.items, &fixed_width), min_width());
    }

    #[test]
    fn checkmarks_shift_the_text_column_and_shortcuts_widen_the_panel() {
        let pos = Point::new(px(0.0), px(0.0));
        let plain =
            ContextMenu::new(pos).item("A very long menu item label here", Box::new(DismissMenu));
        let checked = ContextMenu::new(pos).checked_item(
            "A very long menu item label here",
            MenuCheck::On,
            Box::new(DismissMenu),
        );
        let plain_width = panel_width(&plain.items, &fixed_width);
        let checked_width = panel_width(&checked.items, &fixed_width);
        // Widths round up to whole points.
        assert!((checked_width - plain_width - CHECK_COLUMN).abs() <= 1.0);
        assert_eq!(Columns::of(&checked.items).text_x(), 24.0);
        assert_eq!(Columns::of(&plain.items).text_x(), 16.5);
    }

    #[test]
    fn tag_row_is_taller_and_fits_seven_dots() {
        let pos = Point::new(px(0.0), px(0.0));
        let tags = (0..7)
            .map(|i| MenuTag {
                label: format!("Tag {i}").into(),
                color: gpui::rgb(0xff3b30).into(),
                checked: i == 0,
                action: Box::new(DismissMenu) as Box<dyn Action>,
            })
            .collect();
        let menu = ContextMenu::new(pos).tag_row("label", tags);
        assert_eq!(
            panel_height(&menu.items),
            PANEL_PADDING * 2.0 + TAG_ROW_HEIGHT
        );
        let needed = TEXT_INSET + 7.0 * TAG_TARGET + 6.0 * TAG_GAP + TEXT_INSET;
        assert!(panel_width(&menu.items, &fixed_width) >= needed);
    }

    #[test]
    fn signature_changes_with_a_check_mark() {
        let pos = Point::new(px(0.0), px(0.0));
        let off =
            ContextMenu::new(pos).checked_item("Name", MenuCheck::None, Box::new(DismissMenu));
        let on = ContextMenu::new(pos).checked_item("Name", MenuCheck::On, Box::new(DismissMenu));
        assert_ne!(signature(&off.items), signature(&on.items));
        let again =
            ContextMenu::new(pos).checked_item("Name", MenuCheck::On, Box::new(DismissMenu));
        assert_eq!(signature(&on.items), signature(&again.items));
    }
}
