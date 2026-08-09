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
    ElementId, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent,
    MouseButton, ParentElement as _, Pixels, Point, SharedString, Styled as _, Window,
};
use gpui_component::StyledExt as _;

use crate::{mac, shortcuts::Shortcut, Button, ButtonRole, ListRow};

gpui::actions!(rmac_ui, [DismissMenu, RequestClose]);

const MENU_CONTEXT: &str = "RmacContextMenu";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        crate::shortcuts::ESCAPE.keystroke,
        DismissMenu,
        Some(MENU_CONTEXT),
    )]);
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

/// A macOS pill dialog button. Returns a stateful element the caller wires with
/// `.on_click(cx.listener(...))` and then `.into_any_element()`.
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
    Button::new(id, label).role(role)
}

/// Wrap arbitrary content in a centered modal: a dimmed full-window scrim with
/// the content floated in the middle. Used by [`alert`]; exposed for custom
/// dialogs (e.g. a text-field sheet).
pub fn dialog(id: impl Into<ElementId>, content: impl IntoElement) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(mac::scrim())
        // Swallow clicks on the scrim so they don't fall through to the app.
        .occlude()
        .child(content)
}

/// A standard macOS alert: a centered card with an optional bold title, a
/// secondary message, and a right-aligned row of buttons (default rightmost).
///
/// Render this as the LAST child of the app root, gated on the app's
/// "is a dialog open?" state.
pub fn alert(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> impl IntoElement {
    let title: SharedString = title.into();
    let message: SharedString = message.into();

    let card = div()
        .v_flex()
        .tab_group()
        .w(px(300.0))
        .p(px(20.0))
        .gap_2()
        .rounded(px(mac::radius_popover()))
        .bg(mac::window())
        .border_1()
        .border_color(mac::separator())
        .shadow_xl()
        // Clicks on the card shouldn't dismiss via the scrim.
        .occlude()
        .when(!title.is_empty(), |el| {
            el.child(
                div()
                    .text_size(crate::text_px(15.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(title),
            )
        })
        .when(!message.is_empty(), |el| {
            el.child(
                div()
                    .text_size(crate::text_px(13.0))
                    .text_color(mac::text_secondary())
                    .child(message),
            )
        })
        .child(
            div()
                .h_flex()
                .justify_end()
                .gap_2()
                .pt_2()
                .children(buttons),
        );

    dialog("rmac-alert", card)
}

// ---- Context menu ---------------------------------------------------------

/// One entry in a [`ContextMenu`].
enum MenuEntry {
    Item {
        label: SharedString,
        shortcut: Option<SharedString>,
        action: Box<dyn Action>,
        danger: bool,
    },
    Separator,
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
}

impl ContextMenuState {
    /// Open a menu at `position`, transfer keyboard focus to it, and remember
    /// the invoking control so focus can be restored when it closes.
    pub fn open<V>(
        position: Point<Pixels>,
        return_focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let menu_focus = cx.focus_handle();
        window.focus(&menu_focus);
        Self {
            position,
            menu_focus,
            return_focus: return_focus.clone(),
        }
    }

    /// Cursor-relative position used to anchor the menu.
    pub fn position(&self) -> Point<Pixels> {
        self.position
    }

    /// Close `menu`, returning focus to the control that opened it.
    pub fn dismiss(menu: &mut Option<Self>, window: &mut Window) -> bool {
        let Some(menu) = menu.take() else {
            return false;
        };
        window.focus(&menu.return_focus);
        true
    }
}

fn move_context_menu_focus(
    menu_focus: &FocusHandle,
    forward: bool,
    window: &mut Window,
    cx: &mut App,
) {
    if forward {
        window.focus_next();
        if !menu_focus.contains_focused(window, cx) {
            window.focus(menu_focus);
            window.focus_next();
        }
    } else {
        window.focus_prev();
        if !menu_focus.contains_focused(window, cx) {
            // The menu is rendered last. Starting reverse traversal without a
            // current focus therefore wraps to its final enabled item.
            window.blur();
            window.focus_prev();
        }
    }

    // Empty or fully disabled menus retain focus on their overlay rather than
    // leaking keyboard input to the application underneath.
    if !menu_focus.contains_focused(window, cx) {
        window.focus(menu_focus);
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
        });
        self
    }

    /// Append a thin divider.
    pub fn separator(mut self) -> Self {
        self.items.push(MenuEntry::Separator);
        self
    }

    /// Build the overlay element. Render this as the LAST child of the app root.
    pub fn render(self, state: &ContextMenuState) -> impl IntoElement {
        let pos = self.pos;
        let menu_focus = state.menu_focus.clone();
        let navigation_focus = menu_focus.clone();
        let return_focus = state.return_focus.clone();
        let mut panel = div()
            .min_w(px(190.0))
            .py(px(5.0))
            .tab_group()
            .rounded(px(mac::radius_control()))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .occlude();

        for (i, entry) in self.items.into_iter().enumerate() {
            match entry {
                MenuEntry::Separator => {
                    panel = panel.child(
                        div()
                            .my(px(4.0))
                            .mx(px(8.0))
                            .h(px(1.0))
                            .bg(mac::separator()),
                    );
                }
                MenuEntry::Item {
                    label,
                    shortcut,
                    action,
                    danger,
                } => {
                    let base = if danger { mac::danger() } else { mac::text() };
                    let content = div()
                        .w_full()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .text_color(base)
                        .child(div().child(label))
                        .when_some(shortcut, |el, sc| {
                            el.child(
                                div()
                                    .text_size(crate::text_px(12.0))
                                    .text_color(mac::text_tertiary())
                                    .child(sc),
                            )
                        });
                    let row = ListRow::new(("rmac-menu-item", i), content)
                        .mx(px(5.0))
                        .px(px(8.0))
                        .on_activate({
                            let return_focus = return_focus.clone();
                            move |_, window, cx| {
                                window.focus(&return_focus);
                                window.dispatch_action(action.boxed_clone(), cx);
                                window.dispatch_action(Box::new(DismissMenu), cx);
                            }
                        });
                    panel = panel.child(row);
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
                let forward = match event.keystroke.key.as_str() {
                    "down" => Some(true),
                    "up" => Some(false),
                    "tab" => Some(!event.keystroke.modifiers.shift),
                    _ => None,
                };
                if let Some(forward) = forward {
                    cx.stop_propagation();
                    move_context_menu_focus(&navigation_focus, forward, window, cx);
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
