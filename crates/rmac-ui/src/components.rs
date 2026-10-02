//! Shared macOS-fidelity UI components for the rmac suite.
//!
//! These replace per-app ad-hoc modals and the native GPUI prompt so every app
//! shares one look. They are **presentational** — the app owns the open/close
//! state and wires each button's `on_click(cx.listener(...))`. The component
//! lays out the chrome (scrim, card, button row) from the `mac` design tokens.
//!
//! ## Z-order
//! A dialog is an absolute overlay, so it must be the **LAST child** of the
//! app's root element or opaque siblings paint over it.

use gpui::{
    anchored, deferred, div, prelude::FluentBuilder as _, px, Action, AnyElement, App, Context,
    ElementId, FocusHandle, Hsla, InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent,
    MouseButton, ParentElement as _, Pixels, Point, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Toggled, Window,
};
use gpui_component::StyledExt as _;
use std::{cell::Cell, rc::Rc};

use crate::{mac, shortcuts::Shortcut, Button, ButtonRole, ListRow};

gpui::actions!(
    rmac_ui,
    [
        DismissMenu,
        RequestClose,
        PasteAndMatchStyle,
        MinimizeWindow,
        HideApplication,
        HideOtherApplications,
        QuitApplication
    ]
);

const MENU_CONTEXT: &str = "RmacContextMenu";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        crate::shortcuts::ESCAPE.keystroke,
        DismissMenu,
        Some(MENU_CONTEXT),
    )]);
    // ⌘M, ⌘H, ⌥⌘H and ⌘Q work in every rmac app, as they do in every Mac
    // app. niri hands ⌘-letter keys to the focused app, so each app answers
    // them; these context-free bindings are the fallback an app's own
    // binding for the same keys (System Settings' ⌘M and ⌘Q) still overrides.
    cx.bind_keys([
        KeyBinding::new(crate::shortcuts::QUIT.keystroke, QuitApplication, None),
        KeyBinding::new(crate::shortcuts::MINIMIZE.keystroke, MinimizeWindow, None),
        KeyBinding::new(crate::shortcuts::HIDE.keystroke, HideApplication, None),
        KeyBinding::new(
            crate::shortcuts::HIDE_OTHERS.keystroke,
            HideOtherApplications,
            None,
        ),
    ]);
    cx.on_action(|_: &MinimizeWindow, cx| crate::chrome::minimize_focused_window(cx));
    cx.on_action(|_: &HideApplication, cx| crate::chrome::hide_application(false, cx));
    cx.on_action(|_: &HideOtherApplications, cx| crate::chrome::hide_application(true, cx));
    cx.on_action(|_: &QuitApplication, cx| crate::chrome::quit_application(cx));
    cx.on_action(|_: &PasteAndMatchStyle, cx| {
        // InputState is unstyled. Its ordinary Paste applies the target
        // field's style, including when the menu bar temporarily owns focus.
        crate::menu_target::dispatch_menu_action(Box::new(crate::controls::Paste), cx);
    });
}

/// Visual role of a dialog button (drives fill / text color).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogButtonKind {
    /// Filled blue — the default action (rightmost).
    Primary,
    /// Plain bordered — secondary actions.
    Normal,
    /// Filled red — destructive (Delete, Don't Save…).
    Destructive,
}

/// A macOS pill dialog button: 28 pt tall and fully round, as in NSAlert on
/// macOS 26 (design-lab/chrome.html). Returns a stateful element the caller
/// wires with `.on_click(cx.listener(...))` and then `.into_any_element()`.
///
/// ```ignore
/// dialog_button("ok", "Restore", DialogButtonKind::Primary)
///     .on_click(cx.listener(|this, _, w, cx| this.restore(w, cx)))
///     .into_any_element()
/// ```
pub fn dialog_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: DialogButtonKind,
) -> Button {
    let role = match kind {
        DialogButtonKind::Primary => ButtonRole::Primary,
        DialogButtonKind::Destructive => ButtonRole::Destructive,
        DialogButtonKind::Normal => ButtonRole::Secondary,
    };
    Button::new(id, label)
        .role(role)
        .h(px(rmac_design::Metrics::default().alert_button_height))
        .rounded_full()
}

/// Which end of a dialog's own tab order receives focus when it first
/// appears with focus still outside it.
#[derive(Clone, Copy)]
enum InitialFocus {
    /// The first tab stop — right for arbitrary content such as a form
    /// whose own first field wants the cursor.
    First,
    /// The last tab stop — macOS puts an alert's default action rightmost,
    /// so [`alert_with_icon`] asks for this instead.
    Last,
}

/// Move focus to the next/previous tab stop, wrapping back inside `boundary`
/// rather than letting it escape — the real trap behind both an open
/// [`Dialog`]'s Tab/Shift-Tab handling and [`ContextMenu`]'s.
fn cycle_focus_within(boundary: &FocusHandle, forward: bool, window: &mut Window, cx: &mut App) {
    if forward {
        window.focus_next(cx);
        if !boundary.contains_focused(window, cx) {
            window.focus(boundary, cx);
            window.focus_next(cx);
        }
    } else {
        window.focus_prev(cx);
        if !boundary.contains_focused(window, cx) {
            // Starting reverse traversal without a current focus wraps to the
            // final enabled tab stop, so blur before retrying.
            window.blur();
            window.focus_prev(cx);
        }
    }

    // Empty or fully disabled content keeps focus on the boundary itself
    // rather than leaking keyboard input to whatever is behind it.
    if !boundary.contains_focused(window, cx) {
        window.focus(boundary, cx);
    }
}

/// Bring focus inside `boundary` if it currently isn't — a dialog's "just
/// appeared" initial focus, and its recovery if focus ever ends up outside
/// it. Runs on every render.
///
/// This is deliberately two frames, not one: `window.focus_next`/`focus_prev`
/// read the *last painted* frame's tab stops (`Window::focus_next`), which on
/// the frame a dialog first appears don't include its own content yet — only
/// `window.focus(boundary, ...)` is safe to call before that content has ever
/// been painted. So the first frame focus lands on `boundary` (the dialog's
/// own container, a legitimate landing spot for a screen reader), and once
/// that has been painted at least once, the next frame steps from it onto the
/// real first/last control.
fn enter_dialog_focus(
    boundary: &FocusHandle,
    initial: InitialFocus,
    window: &mut Window,
    cx: &mut App,
) {
    if boundary.is_focused(window) {
        match initial {
            InitialFocus::First => window.focus_next(cx),
            InitialFocus::Last => {
                window.blur();
                window.focus_prev(cx);
            }
        }
        if !boundary.contains_focused(window, cx) {
            window.focus(boundary, cx);
        }
        return;
    }
    if !boundary.contains_focused(window, cx) {
        window.focus(boundary, cx);
    }
}

/// A centered modal: a dimmed, click-swallowing scrim with `content` floated
/// over it. While a `Dialog` is on screen, Tab/Shift-Tab cycle within
/// `content` and can never land back on whatever is behind the scrim — the
/// same real trap [`ContextMenu`] already gives a menu, rather than the
/// ordering-only `tab_group()` this used to rely on. Built by [`dialog`] and
/// [`alert`]; render it as the LAST child of the app root (its wrap-to-first/
/// wrap-to-last both depend on being final in tab order), gated on the app's
/// "is a dialog open?" state.
#[derive(IntoElement)]
pub struct Dialog {
    id: ElementId,
    content: AnyElement,
    role: Role,
    aria_label: Option<SharedString>,
    initial_focus: InitialFocus,
    extra_key_down: Vec<KeyDownListener>,
    attached: bool,
    focus_trap: bool,
}

/// A key handler a dialog runs alongside its own key handling.
type KeyDownListener = Box<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>;

impl Dialog {
    /// Leave focus to a newer dialog displayed above this one.
    pub fn passive(mut self) -> Self {
        self.focus_trap = false;
        self
    }
    /// Place a document sheet directly below the window title bar.
    pub fn attached(mut self) -> Self {
        self.attached = true;
        self
    }
    /// Narrow the dialog's accessible role (`alert_with_icon` uses this for
    /// `Role::AlertDialog`).
    fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Set the dialog's accessible name.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// Land initial focus on the last tab stop (the default action) instead
    /// of the first.
    fn initial_focus_last(mut self) -> Self {
        self.initial_focus = InitialFocus::Last;
        self
    }

    /// Add a key-down handler alongside the dialog's own Tab trap — for a
    /// caller's own Escape-cancels/Return-submits wiring (e.g. the Wi-Fi and
    /// Bluetooth sheets in System Settings).
    pub fn capture_key_down(
        mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.extra_key_down.push(Box::new(listener));
        self
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let boundary = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        if self.focus_trap {
            enter_dialog_focus(&boundary, self.initial_focus, window, cx);
        }
        let navigation_boundary = boundary.clone();

        let mut element = div()
            .id(self.id)
            .role(self.role)
            .absolute()
            .inset_0()
            .flex()
            .when(self.attached, |el| {
                el.items_start()
                    .pt(px(rmac_design::Metrics::default().titlebar_height))
            })
            .when(!self.attached, |el| el.items_center())
            .justify_center()
            .bg(mac::scrim())
            // Swallow clicks on the scrim so they don't fall through to the app.
            .occlude()
            .track_focus(&boundary)
            .when(self.focus_trap, |element| {
                element.capture_key_down(move |event: &KeyDownEvent, window, cx| {
                    let forward = match event.keystroke.key.as_str() {
                        "tab" => Some(!event.keystroke.modifiers.shift),
                        _ => None,
                    };
                    if let Some(forward) = forward {
                        cx.stop_propagation();
                        cycle_focus_within(&navigation_boundary, forward, window, cx);
                    }
                })
            })
            .when_some(self.aria_label, |el, name| el.aria_label(name))
            .child(self.content);
        for listener in self.extra_key_down {
            element = element.capture_key_down(
                move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                    listener(event, window, cx)
                },
            );
        }
        element
    }
}

/// Wrap arbitrary content in a centered modal. Used by [`alert`]; exposed for
/// custom dialogs (e.g. a text-field sheet). See [`Dialog`] for the trap this
/// gives Tab/Shift-Tab.
pub fn dialog(id: impl Into<ElementId>, content: impl IntoElement) -> Dialog {
    Dialog {
        id: id.into(),
        content: content.into_any_element(),
        // `alert_with_icon` narrows this to `AlertDialog` and adds the
        // title as its accessible name.
        role: Role::Dialog,
        aria_label: None,
        initial_focus: InitialFocus::First,
        extra_key_down: Vec::new(),
        attached: false,
        focus_trap: true,
    }
}

/// A standard macOS 26 alert, measured on NSAlert (design-lab/chrome.html):
/// a 260 pt card, 13 pt bold title and 13 pt message in a 180 pt column,
/// and the buttons as equal-width pills side by side (stacked when there
/// are more than two), default rightmost.
///
/// Render this as the LAST child of the app root, gated on the app's
/// "is a dialog open?" state.
/// Returns the underlying [`Dialog`] (rather than an opaque `impl
/// IntoElement`) so a caller that needs it — e.g. Text Editor's Save sheet,
/// where Esc must cancel — can chain [`Dialog::capture_key_down`].
pub fn alert(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> Dialog {
    alert_impl(None, title, message, buttons, true)
}

/// Like [`alert`], but the *first* (leftmost, "Cancel") button carries the
/// keyboard default instead of the last. Apple's caution-alert pattern for
/// an irreversible action (Finder's Delete Immediately, TextEdit's discard
/// sheet, …): the destructive button is still styled red, but an errant
/// Return does not trigger it — Cancel does.
pub fn alert_cancel_default(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> Dialog {
    alert_impl(None, title, message, buttons, false)
}

/// [`alert`] with the app icon drawn 64 pt at the top left, as macOS does.
pub fn alert_with_icon(
    icon: Option<AnyElement>,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> Dialog {
    alert_impl(icon, title, message, buttons, true)
}

fn alert_impl(
    icon: Option<AnyElement>,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
    default_last: bool,
) -> Dialog {
    let title: SharedString = title.into();
    let message: SharedString = message.into();
    // NSAlert's accessible name is its title, falling back to the message for
    // the (rare) title-less alert. Captured before both are moved into the
    // card below.
    let accessible_name = if !title.is_empty() {
        Some(title.clone())
    } else if !message.is_empty() {
        Some(message.clone())
    } else {
        None
    };
    let metrics = rmac_design::Metrics::default();
    let stacked = buttons.len() > 2;
    // Each button sits in a flex cell whose column stretches it to the
    // cell's full width, so two buttons share the row equally.
    let cells = buttons.into_iter().map(|button| {
        div()
            .when(!stacked, |cell| cell.flex_1())
            .flex()
            .flex_col()
            .child(button)
            .into_any_element()
    });

    let card = div()
        .v_flex()
        .tab_group()
        .w(px(metrics.alert_width))
        .pt(px(20.0))
        .px(px(metrics.alert_padding))
        .pb(px(metrics.alert_padding))
        .rounded(px(rmac_design::Radii::default().alert))
        .bg(mac::sheet())
        .border_1()
        .border_color(mac::separator())
        .shadow_xl()
        // Clicks on the card shouldn't dismiss via the scrim.
        .occlude()
        .when_some(icon, |el, icon| {
            el.child(
                div()
                    .pl(px(4.0))
                    .size(px(metrics.alert_icon + 4.0))
                    .child(icon),
            )
        })
        .when(!title.is_empty(), |el| {
            el.child(
                div()
                    .mt(px(16.0))
                    .px(px(6.0))
                    .max_w(px(metrics.alert_text_width + 12.0))
                    .text_size(crate::text_px(13.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(title),
            )
        })
        .when(!message.is_empty(), |el| {
            el.child(
                div()
                    .mt(px(9.0))
                    .px(px(6.0))
                    .max_w(px(metrics.alert_text_width + 12.0))
                    .text_size(crate::text_px(13.0))
                    .text_color(mac::text())
                    .child(message),
            )
        })
        .child(
            div()
                .mt(px(16.0))
                .flex()
                .when(stacked, |row| row.flex_col())
                .gap(px(metrics.alert_button_gap))
                .children(cells),
        );

    dialog("rmac-alert", card)
        .role(Role::AlertDialog)
        .when_some(accessible_name, |el, name| el.aria_label(name))
        // macOS puts most alerts' default action rightmost, so land initial
        // (and wrapped) focus there instead of the leftmost/Cancel button —
        // unless the caller asked for `alert_cancel_default`'s caution-alert
        // behavior, where Cancel (the dialog's own first tab stop) stays
        // the default so an errant Return can't trigger a destructive one.
        .when(default_last, |dialog| dialog.initial_focus_last())
}

// ---- Context menu ---------------------------------------------------------

/// Checkmark column state for a menu row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuCheck {
    #[default]
    None,
    On,
    Mixed,
}

/// One entry in a [`ContextMenu`].
enum MenuEntry {
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
        items: Vec<MenuEntry>,
    },
}

/// First enabled item whose label starts with `query` (case-insensitive).
/// This is the pure model behind macOS type-select.
pub fn type_select_match(labels: &[(&str, bool)], query: &str) -> Option<usize> {
    let query = query.to_lowercase();
    labels
        .iter()
        .position(|(label, enabled)| *enabled && label.to_lowercase().starts_with(&query))
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
    owner: gpui::EntityId,
    submenu_opens_left: bool,
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
        Self {
            position,
            menu_focus,
            return_focus: return_focus.clone(),
            active_submenu: Rc::new(Cell::new(None)),
            owner: cx.entity_id(),
            // The parent menu has a 190 px minimum width; reserve the same
            // space for its flyout and keep an 8 px window margin.
            submenu_opens_left: position.x + px(380.0) > window.bounds().size.width - px(8.0),
        }
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

/// A macOS-style right-click context menu. Open a [`ContextMenuState`] in a
/// right-mouse handler, store that state in the app, and render this menu as the
/// LAST child of the app root. Clicking an item dispatches its action and the
/// [`DismissMenu`] action; clicking away or pressing Escape dispatches
/// [`DismissMenu`]. The app binds `DismissMenu` to dismiss its menu state.
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

    /// Append a normal item that dispatches `action` when chosen.
    pub fn item(mut self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: None,
            action,
            danger: false,
            enabled: true,
            checked: MenuCheck::None,
            swatch: None,
        });
        self
    }

    /// Append an item showing a right-aligned shortcut hint (e.g. "⌘C").
    fn item_shortcut(
        mut self,
        label: impl Into<SharedString>,
        shortcut: impl Into<SharedString>,
        action: Box<dyn Action>,
    ) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: Some(shortcut.into()),
            action,
            danger: false,
            enabled: true,
            checked: MenuCheck::None,
            swatch: None,
        });
        self
    }

    /// Append a normal item using the shared binding and menu hint.
    pub fn command_item(
        self,
        label: impl Into<SharedString>,
        shortcut: Shortcut,
        action: Box<dyn Action>,
    ) -> Self {
        self.item_shortcut(label, shortcut.hint, action)
    }

    /// Append a destructive item (red label, e.g. Delete / Move to Trash).
    pub fn danger_item(mut self, label: impl Into<SharedString>, action: Box<dyn Action>) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: None,
            action,
            danger: true,
            enabled: true,
            checked: MenuCheck::None,
            swatch: None,
        });
        self
    }

    /// Append a destructive item using the shared binding and menu hint.
    pub fn danger_command_item(
        mut self,
        label: impl Into<SharedString>,
        shortcut: Shortcut,
        action: Box<dyn Action>,
    ) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: Some(shortcut.hint.into()),
            action,
            danger: true,
            enabled: true,
            checked: MenuCheck::None,
            swatch: None,
        });
        self
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
    pub fn disabled_item(
        mut self,
        label: impl Into<SharedString>,
        action: Box<dyn Action>,
    ) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: None,
            action,
            danger: false,
            enabled: false,
            checked: MenuCheck::None,
            swatch: None,
        });
        self
    }

    /// Append an item with a checkmark (`On`) or dash (`Mixed`) in the check
    /// column, as used by View menus.
    pub fn checked_item(
        mut self,
        label: impl Into<SharedString>,
        checked: MenuCheck,
        action: Box<dyn Action>,
    ) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: None,
            action,
            danger: false,
            enabled: true,
            checked,
            swatch: None,
        });
        self
    }

    /// Append a checked item with a color swatch separate from its check mark.
    /// The item's accessible name remains `label`; the swatch is named as a
    /// child image so assistive technology can distinguish its color purpose.
    pub fn checked_item_with_swatch(
        mut self,
        label: impl Into<SharedString>,
        checked: MenuCheck,
        swatch: Hsla,
        action: Box<dyn Action>,
    ) -> Self {
        self.items.push(MenuEntry::Item {
            label: label.into(),
            shortcut: None,
            action,
            danger: false,
            enabled: true,
            checked,
            swatch: Some(swatch),
        });
        self
    }

    /// Append a flyout menu. Choosing its row opens the flyout without
    /// dismissing the parent menu; child actions dismiss the whole menu.
    pub fn submenu(mut self, label: impl Into<SharedString>, submenu: ContextMenu) -> Self {
        self.items.push(MenuEntry::Submenu {
            label: label.into(),
            items: submenu.items,
        });
        self
    }

    /// Build the overlay element. Render this as the LAST child of the app root.
    pub fn render(self, state: &ContextMenuState) -> impl IntoElement {
        let pos = self.pos;
        let menu_focus = state.menu_focus.clone();
        let navigation_focus = menu_focus.clone();
        let return_focus = state.return_focus.clone();
        let active_submenu = state.active_submenu.clone();
        let owner = state.owner;
        let submenu_opens_left = state.submenu_opens_left;
        let has_swatch_column = self.items.iter().any(|entry| {
            matches!(
                entry,
                MenuEntry::Item {
                    swatch: Some(_),
                    ..
                }
            )
        });
        let mut panel = div()
            .id("rmac-context-menu")
            .role(Role::Menu)
            .min_w(px(190.0))
            .py(px(5.0))
            .tab_group()
            .rounded(px(mac::radius_card()))
            .bg(mac::material())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .occlude();

        for (i, entry) in self.items.into_iter().enumerate() {
            match entry {
                MenuEntry::Separator => {
                    panel = panel.child(
                        div()
                            .id(("rmac-context-menu-separator", i))
                            .role(Role::Splitter)
                            .my(px(4.0))
                            .mx(px(8.0))
                            .h(px(1.0))
                            .bg(mac::separator()),
                    );
                }
                MenuEntry::Header(label) => {
                    panel = panel.child(
                        div()
                            .mx(px(5.0))
                            .px(px(if has_swatch_column { 20.0 } else { 8.0 }))
                            .py(px(3.0))
                            .text_size(crate::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_secondary())
                            .child(label),
                    );
                }
                MenuEntry::Submenu { label, items } => {
                    let active = active_submenu.get() == Some(i);
                    let accessible_label = label.clone();
                    let child_focus = menu_focus.clone();
                    let child_return_focus = return_focus.clone();
                    let submenu_children =
                        render_submenu_entries(items, i, child_focus, child_return_focus);
                    let mut row = ListRow::new(
                        ("rmac-menu-submenu", i),
                        div()
                            .w_full()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(14.0)))
                            .when(has_swatch_column, |el| el.child(div().w(px(12.0))))
                            .child(div().flex_1().child(label))
                            .child(div().text_color(mac::text_tertiary()).child("›")),
                    )
                    .mx(px(5.0))
                    .px(px(8.0))
                    .role(Role::MenuItem)
                    .aria_label(accessible_label)
                    .aria_expanded(active);
                    row = row.on_activate({
                        let active_submenu = active_submenu.clone();
                        move |_, _, cx| {
                            active_submenu.set(Some(i));
                            cx.notify(owner);
                        }
                    });
                    let move_active = active_submenu.clone();
                    let move_owner = owner;
                    let click_active = active_submenu.clone();
                    let click_owner = owner;
                    let flyout = div()
                        .id(("rmac-menu-submenu-panel", i))
                        .role(Role::Menu)
                        .absolute()
                        .top_0()
                        .min_w(px(190.0))
                        .py(px(5.0))
                        .tab_group()
                        .rounded(px(mac::radius_card()))
                        .bg(mac::material())
                        .border_1()
                        .border_color(mac::separator())
                        .shadow_lg()
                        .occlude()
                        .children(submenu_children);
                    let flyout = if submenu_opens_left {
                        flyout.right(px(190.0))
                    } else {
                        flyout.left_full()
                    };
                    panel = panel.child(
                        div()
                            .id(("rmac-menu-submenu-row", i))
                            .relative()
                            .w_full()
                            .on_mouse_move(move |_, _, cx| {
                                if move_active.get() != Some(i) {
                                    move_active.set(Some(i));
                                    cx.notify(move_owner);
                                }
                            })
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                click_active.set(Some(i));
                                cx.notify(click_owner);
                            })
                            .child(row)
                            .when(active, |el| el.child(flyout)),
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
                    let base = if !enabled {
                        mac::text_tertiary()
                    } else if danger {
                        mac::danger()
                    } else {
                        mac::text()
                    };
                    let mark = match checked {
                        MenuCheck::On => "✓",
                        MenuCheck::Mixed => "–",
                        MenuCheck::None => "",
                    };
                    // The visible row also carries the checkmark glyph and a
                    // shortcut hint as sibling text runs, so a content-derived
                    // name would read them out too (e.g. "✓ Show Hidden
                    // Files ⌘⇧."). Every item gets an explicit name of just
                    // its label instead.
                    let accessible_label = label.clone();
                    let swatch_accessible_label = format!("{label} tag color");
                    let content = div()
                        .w_full()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .text_color(base)
                        .child(
                            div()
                                .w(px(14.0))
                                .flex()
                                .justify_center()
                                .text_size(crate::text_px(12.0))
                                .child(mark),
                        )
                        .when(has_swatch_column, |el| {
                            el.child(
                                div()
                                    .w(px(12.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when_some(swatch, |el, color| {
                                        el.child(
                                            div()
                                                .id(("rmac-context-menu-swatch", i))
                                                .role(Role::Image)
                                                .aria_label(swatch_accessible_label)
                                                .w(px(9.0))
                                                .h(px(9.0))
                                                .rounded_full()
                                                .bg(color),
                                        )
                                    }),
                            )
                        })
                        .child(div().flex_1().child(label))
                        .when_some(shortcut, |el, sc| {
                            el.child(
                                div()
                                    .text_size(crate::text_px(12.0))
                                    .text_color(mac::text_tertiary())
                                    .child(sc),
                            )
                        });
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
                    let mut row = ListRow::new(("rmac-menu-item", i), content)
                        .mx(px(5.0))
                        .px(px(8.0))
                        .disabled(!enabled)
                        .role(role)
                        .aria_label(accessible_label);
                    if let Some(toggled) = toggled {
                        row = row.aria_toggled(toggled);
                    }
                    let hover_submenu = active_submenu.clone();
                    let hover_owner = owner;
                    if enabled {
                        row = row.on_activate({
                            let return_focus = return_focus.clone();
                            move |_, window, cx| {
                                window.focus(&return_focus, cx);
                                // Dismiss first so actions that deliberately
                                // move focus (for example Finder's inline
                                // Rename editor) keep their new focus.
                                window.dispatch_action(Box::new(DismissMenu), cx);
                                window.dispatch_action(action.boxed_clone(), cx);
                            }
                        });
                    }
                    panel = panel.child(
                        div()
                            .on_mouse_move(move |_, _, cx| {
                                if hover_submenu.get().is_some() {
                                    hover_submenu.set(None);
                                    cx.notify(hover_owner);
                                }
                            })
                            .child(row),
                    );
                }
            }
        }

        // Full-window click-away catcher beneath the panel.
        div()
            .absolute()
            .inset_0()
            .id("rmac-menu-scrim")
            .track_focus(&menu_focus)
            .key_context(MENU_CONTEXT)
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() == "escape" && active_submenu.get().is_some() {
                    cx.stop_propagation();
                    active_submenu.set(None);
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
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.dispatch_action(Box::new(DismissMenu), cx);
            })
            .on_mouse_down(MouseButton::Right, |_, window, cx| {
                window.dispatch_action(Box::new(DismissMenu), cx);
            })
            // Anchor the panel at the cursor but clamp it inside the window so a
            // menu opened near the bottom/right edge never renders off-screen.
            .child(
                deferred(
                    anchored()
                        .position(pos)
                        .snap_to_window_with_margin(px(8.0))
                        .child(panel),
                )
                .with_priority(1),
            )
    }
}

fn render_submenu_entries(
    entries: Vec<MenuEntry>,
    parent: usize,
    _menu_focus: FocusHandle,
    return_focus: FocusHandle,
) -> Vec<AnyElement> {
    let has_swatch_column = entries.iter().any(|entry| {
        matches!(
            entry,
            MenuEntry::Item {
                swatch: Some(_),
                ..
            }
        )
    });
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let id = format!("{parent}-{index}");
            match entry {
                MenuEntry::Separator => Some(
                    div()
                        .id(format!("rmac-submenu-separator-{id}"))
                        .role(Role::Splitter)
                        .my(px(4.0))
                        .mx(px(8.0))
                        .h(px(1.0))
                        .bg(mac::separator())
                        .into_any_element(),
                ),
                MenuEntry::Header(label) => Some(
                    div()
                        .mx(px(5.0))
                        .px(px(if has_swatch_column { 20.0 } else { 8.0 }))
                        .py(px(3.0))
                        .text_size(crate::text_px(11.0))
                        .font_weight(mac::SEMIBOLD)
                        .text_color(mac::text_secondary())
                        .child(label)
                        .into_any_element(),
                ),
                MenuEntry::Item {
                    label,
                    shortcut,
                    action,
                    danger,
                    enabled,
                    checked,
                    swatch,
                } => {
                    let base = if !enabled {
                        mac::text_tertiary()
                    } else if danger {
                        mac::danger()
                    } else {
                        mac::text()
                    };
                    let mark = match checked {
                        MenuCheck::On => "✓",
                        MenuCheck::Mixed => "–",
                        MenuCheck::None => "",
                    };
                    let accessible_label = label.clone();
                    let swatch_accessible_label = format!("{label} tag color");
                    let content = div()
                        .w_full()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .text_color(base)
                        .child(
                            div()
                                .w(px(14.0))
                                .flex()
                                .justify_center()
                                .text_size(crate::text_px(12.0))
                                .child(mark),
                        )
                        .when(has_swatch_column, |el| {
                            el.child(
                                div()
                                    .w(px(12.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when_some(swatch, |el, color| {
                                        el.child(
                                            div()
                                                .id(format!("rmac-submenu-swatch-{id}"))
                                                .role(Role::Image)
                                                .aria_label(swatch_accessible_label)
                                                .w(px(9.0))
                                                .h(px(9.0))
                                                .rounded_full()
                                                .bg(color),
                                        )
                                    }),
                            )
                        })
                        .child(div().flex_1().child(label))
                        .when_some(shortcut, |el, sc| {
                            el.child(
                                div()
                                    .text_size(crate::text_px(12.0))
                                    .text_color(mac::text_tertiary())
                                    .child(sc),
                            )
                        });
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
                    let mut row = ListRow::new(format!("rmac-submenu-item-{id}"), content)
                        .mx(px(5.0))
                        .px(px(8.0))
                        .disabled(!enabled)
                        .role(role)
                        .aria_label(accessible_label);
                    if let Some(toggled) = toggled {
                        row = row.aria_toggled(toggled);
                    }
                    if enabled {
                        row = row.on_activate({
                            let return_focus = return_focus.clone();
                            move |_, window, cx| {
                                window.focus(&return_focus, cx);
                                window.dispatch_action(Box::new(DismissMenu), cx);
                                window.dispatch_action(action.boxed_clone(), cx);
                            }
                        });
                    }
                    Some(row.into_any_element())
                }
                MenuEntry::Submenu { .. } => None,
            }
        })
        .collect()
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
}
