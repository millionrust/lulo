//! A menu opened from the bar. It draws in a clear panel window covering
//! the screen below the bar, so a click anywhere else closes it (as on the
//! Mac) without reaching the window under it, and the panel takes the
//! keyboard while it is open (↑/↓, Return, Esc). Closing gives the
//! keyboard back to the app in front.

use gpui::{
    div, point, px, App, Context, Entity, FocusHandle, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Point, Render, SharedString, Styled as _,
    Subscription, Window,
};
use rmac_app_menu::{CheckState, Item};
use rmac_ui::{ContextMenu, ContextMenuState, DismissMenu, MenuCheck};
use windows::Win32::Foundation::RECT;

use super::{runtime, shell, Surface};
use crate::model::menus;
use crate::win::{surface, trace, windows_list};

/// A command chosen in a bar menu: one of the shell's own (the Lulo menu,
/// a Windows app's menus) or one for the front Lulo app.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = lulo_shell, no_json)]
struct RunMenuCommand {
    name: SharedString,
    app: bool,
}

struct OverlayEntity(Entity<MenuOverlay>);

impl Global for OverlayEntity {}

pub(crate) struct MenuOverlay {
    focus: FocusHandle,
    menu: Option<OpenMenu>,
    _activation: Subscription,
}

struct OpenMenu {
    state: ContextMenuState,
    items: Vec<Item>,
    app: bool,
}

fn context_menu(position: Point<Pixels>, items: &[Item], app: bool) -> ContextMenu {
    let mut menu = ContextMenu::new(position);
    for (index, item) in items.iter().enumerate() {
        if item.separator_before && index > 0 {
            menu = menu.separator();
        }
        menu = if item.is_submenu() {
            if item.enabled {
                menu.submenu(
                    item.label.clone(),
                    context_menu(position, &item.children, app),
                )
            } else {
                menu.entry(
                    item.label.clone(),
                    None,
                    Box::new(DismissMenu),
                    false,
                    MenuCheck::None,
                )
            }
        } else {
            let check = match item.checked {
                CheckState::Off => MenuCheck::None,
                CheckState::On => MenuCheck::On,
                CheckState::Mixed => MenuCheck::Mixed,
            };
            let shortcut =
                (!item.shortcut.is_empty()).then(|| SharedString::from(item.shortcut.clone()));
            menu.entry(
                item.label.clone(),
                shortcut,
                Box::new(RunMenuCommand {
                    name: item.action.clone().into(),
                    app,
                }),
                item.enabled,
                check,
            )
        };
    }
    menu
}

impl MenuOverlay {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let entity = cx.entity();
        cx.set_global(OverlayEntity(entity));
        // Switching to another app with the keyboard closes the menu.
        let activation = cx.observe_window_activation(window, |overlay, window, cx| {
            if !window.is_window_active() && overlay.menu.is_some() {
                cx.defer(|cx| close(false, cx));
            }
        });
        Self {
            focus: cx.focus_handle(),
            menu: None,
            _activation: activation,
        }
    }

    fn show(
        &mut self,
        items: Vec<Item>,
        app: bool,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = ContextMenuState::open(position, &self.focus, window, cx);
        self.menu = Some(OpenMenu { state, items, app });
        cx.notify();
    }
}

impl Render for MenuOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("lulo-menu-overlay")
            .size_full()
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &DismissMenu, _, cx| {
                cx.defer(|cx| close(true, cx));
            }))
            .on_action(cx.listener(|_, command: &RunMenuCommand, _, cx| {
                let command = command.clone();
                cx.defer(move |cx| {
                    trace(|| format!("menu command {}", command.name));
                    close(!command.app, cx);
                    if command.app {
                        super::run_app_command(&command.name, cx);
                    } else {
                        super::run_shell_command(&command.name, cx);
                    }
                });
            }))
            .children(self.menu.as_ref().map(|menu| {
                context_menu(menu.state.position(), &menu.items, menu.app).render(&menu.state)
            }))
    }
}

/// The items of bar title `index`, and whether they belong to the front
/// Lulo app.
fn items(index: usize, cx: &App) -> Option<(Vec<Item>, bool)> {
    let state = shell(cx).read(cx);
    if index == 0 {
        let menu = menus::lulo_menu(
            &crate::win::user_name(),
            state.starts_at_sign_in,
            state.files_for_folders,
        );
        return Some((menu.items, false));
    }
    let app = state.front_is_lulo();
    state
        .front_menus()
        .into_iter()
        .nth(index - 1)
        .map(|menu| (menu.items, app))
}

fn overlay_rect(cx: &App) -> RECT {
    let (monitor, _) = surface::primary_monitor();
    RECT {
        top: runtime(cx).bar_rect.bottom.max(monitor.top),
        ..monitor
    }
}

/// Open bar title `index`'s menu below it (`left` is the title's left edge
/// in the bar's coordinates).
pub(crate) fn open(index: usize, left: f32, cx: &mut App) {
    let Some((items, app)) = items(index, cx) else {
        return;
    };
    if app {
        // The menu opens at once with the menus last sent; the app's fresh
        // state replaces them a moment later, as `Layout` does on Linux.
        super::validate_front_menus(cx);
    }
    show_overlay(index, items, app, point(px(left), px(1.0)), cx);
}

/// `open_menu`'s value while a Dock tile's menu is open.
pub(crate) const DOCK_MENU: usize = usize::MAX;

/// Open a Dock tile's menu with its top-left corner at `position` on the
/// screen (physical pixels).
pub(crate) fn open_dock_menu(items: Vec<Item>, position: (i32, i32), cx: &mut App) {
    let rect = overlay_rect(cx);
    let scale = runtime(cx).scale;
    let at = point(
        px((position.0 - rect.left) as f32 / scale),
        px((position.1 - rect.top) as f32 / scale),
    );
    show_overlay(DOCK_MENU, items, false, at, cx);
}

fn show_overlay(index: usize, items: Vec<Item>, app: bool, at: Point<Pixels>, cx: &mut App) {
    super::keep_panel(super::Panel::Menu, cx);
    let overlay = match runtime(cx).overlay {
        Some(overlay) => overlay,
        None => {
            let rect = overlay_rect(cx);
            let scale = runtime(cx).scale;
            let Some(overlay) =
                super::open_surface(super::logical(rect, scale), true, cx, MenuOverlay::new)
            else {
                return;
            };
            cx.global_mut::<super::Runtime>().overlay = Some(overlay);
            overlay
        }
    };
    let Some(entity) = cx
        .try_global::<OverlayEntity>()
        .map(|overlay| overlay.0.clone())
    else {
        return;
    };
    let _ = overlay.handle.update(cx, |_, window, cx| {
        entity.update(cx, |view, cx| view.show(items, app, at, window, cx));
    });
    // Shown once it holds the new menu, so no frame of the last one shows.
    let rect = overlay_rect(cx);
    super::later(cx, move || {
        surface::show_focused_at(windows_list::handle(overlay.hwnd), rect)
    });
    shell(cx).update(cx, |state, cx| {
        state.open_menu = Some(index);
        cx.notify();
    });
    if index == DOCK_MENU {
        trace(|| "menu dock open".into());
    } else {
        trace(|| format!("menu {index} open"));
    }
}

/// Show the front Lulo app's freshly validated items in its open menu.
pub(crate) fn refresh(cx: &mut App) {
    let Some(index) = shell(cx).read(cx).open_menu.filter(|&index| index > 0) else {
        return;
    };
    let Some((items, app)) = items(index, cx) else {
        return;
    };
    if let Some(entity) = cx
        .try_global::<OverlayEntity>()
        .map(|overlay| overlay.0.clone())
    {
        entity.update(cx, |view, cx| {
            if let Some(menu) = view.menu.as_mut() {
                menu.items = items;
                menu.app = app;
                cx.notify();
            }
        });
    }
}

/// Close the open menu; with `give_back`, the app in front gets the
/// keyboard again.
pub(crate) fn close(give_back: bool, cx: &mut App) {
    let shell = shell(cx);
    if shell.read(cx).open_menu.is_none() {
        return;
    }
    let overlay = runtime(cx).overlay.map(|Surface { hwnd, .. }| hwnd);
    if let Some(entity) = cx
        .try_global::<OverlayEntity>()
        .map(|overlay| overlay.0.clone())
    {
        entity.update(cx, |view, cx| {
            view.menu = None;
            cx.notify();
        });
    }
    // The app in front gets the keyboard back; with the desktop in front
    // (Lulo mode), Lulo's desktop does.
    let desktop = runtime(cx).desktop.map(|desktop| desktop.hwnd);
    let front = shell.update(cx, |state, cx| {
        state.open_menu = None;
        cx.notify();
        state.front.as_ref().map(|front| front.hwnd).or(desktop)
    });
    super::later(cx, move || {
        if let Some(overlay) = overlay {
            surface::hide(windows_list::handle(overlay));
        }
        if give_back {
            if let Some(hwnd) = front {
                windows_list::activate(hwnd);
            }
        }
    });
    trace(|| "menu closed".into());
    super::release_later(super::Panel::Menu, cx);
}

/// Let go of the menu panel's window a while after the last menu closed.
pub(crate) fn release(cx: &mut App) {
    if shell(cx).read(cx).open_menu.is_some() {
        return;
    }
    let Some(overlay) = cx.global_mut::<super::Runtime>().overlay.take() else {
        return;
    };
    if cx.has_global::<OverlayEntity>() {
        cx.remove_global::<OverlayEntity>();
    }
    let _ = overlay
        .handle
        .update(cx, |_, window, _| window.remove_window());
    trace(|| "menu panel released".into());
}
