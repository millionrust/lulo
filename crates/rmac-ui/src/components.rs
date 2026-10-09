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
    div, prelude::FluentBuilder as _, px, AnyElement, App, ElementId, FocusHandle,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, ParentElement as _, RenderOnce,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use gpui_component::StyledExt as _;
use std::cell::RefCell;

use crate::{mac, Button, ButtonRole};

gpui::actions!(
    rmac_ui,
    [
        DismissMenu,
        RequestClose,
        PasteAndMatchStyle,
        MinimizeWindow,
        ZoomWindow,
        HideApplication,
        HideOtherApplications,
        QuitApplication,
        CycleThroughWindows
    ]
);

pub(crate) const MENU_CONTEXT: &str = "RmacContextMenu";

pub(crate) fn init(cx: &mut App) {
    crate::context_menu::init(cx);
    cx.bind_keys([KeyBinding::new(
        crate::shortcuts::ESCAPE.keystroke,
        DismissMenu,
        Some(MENU_CONTEXT),
    )]);
    // ⌘M, ⌘H, ⌥⌘H and ⌘Q work in every rmac app, as they do in every Mac
    // app. niri hands ⌘-letter keys to the focused app, so each app answers
    // them; these context-free bindings are the fallback an app's own
    // binding for the same keys (System Settings' ⌘M and ⌘Q) still overrides.
    // On Windows they are Ctrl+Q, Ctrl+M… as the menus show, not Win+M.
    crate::shortcuts::bind_keys(
        cx,
        [
            KeyBinding::new(crate::shortcuts::QUIT.keystroke, QuitApplication, None),
            KeyBinding::new(crate::shortcuts::MINIMIZE.keystroke, MinimizeWindow, None),
            KeyBinding::new(crate::shortcuts::ZOOM_WINDOW.keystroke, ZoomWindow, None),
            KeyBinding::new(crate::shortcuts::HIDE.keystroke, HideApplication, None),
            KeyBinding::new(
                crate::shortcuts::HIDE_OTHERS.keystroke,
                HideOtherApplications,
                None,
            ),
            KeyBinding::new(
                crate::shortcuts::CYCLE_THROUGH_WINDOWS.keystroke,
                CycleThroughWindows,
                None,
            ),
        ],
    );
    cx.on_action(|_: &MinimizeWindow, cx| crate::chrome::minimize_focused_window(cx));
    cx.on_action(|_: &ZoomWindow, cx| crate::chrome::zoom_focused_window(cx));
    cx.on_action(|_: &HideApplication, cx| crate::chrome::hide_application(false, cx));
    cx.on_action(|_: &HideOtherApplications, cx| crate::chrome::hide_application(true, cx));
    cx.on_action(|_: &QuitApplication, cx| crate::chrome::quit_application(cx));
    cx.on_action(|_: &CycleThroughWindows, cx| crate::chrome::cycle_through_windows(cx));
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
pub(crate) fn cycle_focus_within(
    boundary: &FocusHandle,
    forward: bool,
    window: &mut Window,
    cx: &mut App,
) {
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

/// Remember the control that had focus before the modal appeared. GPUI drops
/// keyed element state when the dialog is removed from the next frame; that
/// release is the one place shared by Escape, a Cancel click, and every other
/// way an app can dismiss a dialog.
struct DialogFocusState {
    boundary: FocusHandle,
    restore: RefCell<Option<FocusHandle>>,
    last_focus: RefCell<Option<FocusHandle>>,
    _release: Subscription,
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
    restore_focus: Option<FocusHandle>,
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

    /// Return focus to this control when the dialog closes. Use this when an
    /// app focuses a sheet field before its first render, so the automatically
    /// captured focus is already inside the sheet.
    pub fn restore_focus_to(mut self, handle: FocusHandle) -> Self {
        self.restore_focus = Some(handle);
        self
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let boundary = cx.focus_handle();
            let restore = window.focused(cx);
            let release = cx.on_release_in(window, |state: &mut DialogFocusState, window, cx| {
                let focused = window.focused(cx);
                if focused.is_none()
                    || state.boundary.contains_focused(window, cx)
                    || focused == *state.last_focus.get_mut()
                {
                    if let Some(restore) = state.restore.get_mut() {
                        window.focus(restore, cx);
                    }
                }
            });
            DialogFocusState {
                boundary,
                restore: RefCell::new(restore),
                last_focus: RefCell::new(None),
                _release: release,
            }
        });
        let boundary = state.read(cx).boundary.clone();
        if let Some(restore) = self.restore_focus {
            *state.read(cx).restore.borrow_mut() = Some(restore);
        }
        if self.focus_trap {
            enter_dialog_focus(&boundary, self.initial_focus, window, cx);
        }
        *state.read(cx).last_focus.borrow_mut() = window.focused(cx);
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
        restore_focus: None,
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
