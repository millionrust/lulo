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
    MouseButton, ParentElement as _, Pixels, Point, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Toggled, Window,
};
use gpui_component::StyledExt as _;

use crate::{mac, shortcuts::Shortcut, Button, ButtonRole, ListRow};

gpui::actions!(
    rmac_ui,
    [
        DismissMenu,
        RequestClose,
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

/// Wrap arbitrary content in a centered modal: a dimmed full-window scrim with
/// the content floated in the middle. Used by [`alert`]; exposed for custom
/// dialogs (e.g. a text-field sheet).
pub fn dialog(id: impl Into<ElementId>, content: impl IntoElement) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        // Identifies the whole overlay as a dialog surface; `alert_with_icon`
        // narrows this to `AlertDialog` and adds the title as its name.
        .role(Role::Dialog)
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

/// A standard macOS 26 alert, measured on NSAlert (design-lab/chrome.html):
/// a 260 pt card, 13 pt bold title and 13 pt message in a 180 pt column,
/// and the buttons as equal-width pills side by side (stacked when there
/// are more than two), default rightmost.
///
/// Render this as the LAST child of the app root, gated on the app's
/// "is a dialog open?" state.
pub fn alert(
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> impl IntoElement {
    alert_with_icon(None, title, message, buttons)
}

/// [`alert`] with the app icon drawn 64 pt at the top left, as macOS does.
pub fn alert_with_icon(
    icon: Option<AnyElement>,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> impl IntoElement {
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
    },
    Separator,
    Header(SharedString),
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
        window.focus(&menu_focus, cx);
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
    pub fn dismiss(menu: &mut Option<Self>, window: &mut Window, cx: &mut App) -> bool {
        let Some(menu) = menu.take() else {
            return false;
        };
        window.focus(&menu.return_focus, cx);
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
        window.focus_next(cx);
        if !menu_focus.contains_focused(window, cx) {
            window.focus(menu_focus, cx);
            window.focus_next(cx);
        }
    } else {
        window.focus_prev(cx);
        if !menu_focus.contains_focused(window, cx) {
            // The menu is rendered last. Starting reverse traversal without a
            // current focus therefore wraps to its final enabled item.
            window.blur();
            window.focus_prev(cx);
        }
    }

    // Empty or fully disabled menus retain focus on their overlay rather than
    // leaking keyboard input to the application underneath.
    if !menu_focus.contains_focused(window, cx) {
        window.focus(menu_focus, cx);
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
        });
        self
    }

    /// Build the overlay element. Render this as the LAST child of the app root.
    pub fn render(self, state: &ContextMenuState) -> impl IntoElement {
        let pos = self.pos;
        let menu_focus = state.menu_focus.clone();
        let navigation_focus = menu_focus.clone();
        let return_focus = state.return_focus.clone();
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
                            .px(px(8.0))
                            .py(px(3.0))
                            .text_size(crate::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_secondary())
                            .child(label),
                    );
                }
                MenuEntry::Item {
                    label,
                    shortcut,
                    action,
                    danger,
                    enabled,
                    checked,
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
}
