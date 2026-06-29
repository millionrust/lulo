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
    div, prelude::FluentBuilder as _, px, Action, AnyElement, ElementId,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, Point,
    SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::StyledExt as _;

use crate::mac;

gpui::actions!(rmac_ui, [DismissMenu, RequestClose]);

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
) -> gpui::Stateful<gpui::Div> {
    let label: SharedString = label.into();
    let (bg, fg, border) = match kind {
        DialogButtonKind::Primary => (mac::accent(), mac::on_accent(), mac::accent()),
        DialogButtonKind::Destructive => (mac::danger(), mac::on_accent(), mac::danger()),
        DialogButtonKind::Normal => (mac::window(), mac::text(), mac::separator()),
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(28.0))
        .px_4()
        .min_w(px(74.0))
        .rounded(px(7.0))
        .bg(bg)
        .text_color(fg)
        .border_1()
        .border_color(border)
        .text_size(px(13.0))
        .font_weight(mac::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .child(label)
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
        .w(px(300.0))
        .p(px(20.0))
        .gap_2()
        .rounded(px(12.0))
        .bg(mac::window())
        .border_1()
        .border_color(mac::separator())
        .shadow_xl()
        // Clicks on the card shouldn't dismiss via the scrim.
        .occlude()
        .when(!title.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(15.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(title),
            )
        })
        .when(!message.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(13.0))
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

/// A macOS-style right-click context menu. Build it in a right-mouse handler,
/// store it in app state (`Option<ContextMenu>`), and render it as the LAST
/// child of the app root. Clicking an item dispatches its action and the
/// [`DismissMenu`] action; clicking away dispatches [`DismissMenu`]. The app
/// binds `DismissMenu` to clear its menu state.
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
        Self { pos, items: Vec::new() }
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
    pub fn item_shortcut(
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

    /// Append a thin divider.
    pub fn separator(mut self) -> Self {
        self.items.push(MenuEntry::Separator);
        self
    }

    /// Build the overlay element. Render this as the LAST child of the app root.
    pub fn render(self) -> impl IntoElement {
        let mut panel = div()
            .absolute()
            .left(self.pos.x)
            .top(self.pos.y)
            .min_w(px(190.0))
            .py(px(5.0))
            .rounded(px(8.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .occlude();

        for (i, entry) in self.items.into_iter().enumerate() {
            match entry {
                MenuEntry::Separator => {
                    panel = panel.child(
                        div().my(px(4.0)).mx(px(8.0)).h(px(1.0)).bg(mac::separator()),
                    );
                }
                MenuEntry::Item {
                    label,
                    shortcut,
                    action,
                    danger,
                } => {
                    let base = if danger { mac::danger() } else { mac::text() };
                    let row = div()
                        .id(("rmac-menu-item", i))
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .h(px(24.0))
                        .mx(px(5.0))
                        .px(px(8.0))
                        .rounded(px(5.0))
                        .text_size(px(13.0))
                        .text_color(base)
                        .cursor_pointer()
                        .hover(|s| s.bg(mac::accent()).text_color(mac::on_accent()))
                        .child(div().child(label))
                        .when_some(shortcut, |el, sc| {
                            el.child(
                                div().text_size(px(12.0)).text_color(mac::text_tertiary()).child(sc),
                            )
                        })
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(action.boxed_clone(), cx);
                            window.dispatch_action(Box::new(DismissMenu), cx);
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
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.dispatch_action(Box::new(DismissMenu), cx);
            })
            .on_mouse_down(MouseButton::Right, |_, window, cx| {
                window.dispatch_action(Box::new(DismissMenu), cx);
            })
            .child(panel)
    }
}
