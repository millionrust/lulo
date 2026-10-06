//! The in-window menu strip (ADR 0023 phase 2).
//!
//! On Lulo OS an app's menus live in the desktop menu bar, which reads them
//! over D-Bus (`app_menu`). Windows has no Lulo menu bar until phase 3, so
//! each app window draws the same menus itself: a Mac-style strip along the
//! window's top edge, the bold app name first, then the app's menus with
//! the standard Window and Help menus. It shows the same `rmac-app-menu`
//! table with the same live state (`app_menu::current_menus`), opens each
//! menu with the shared [`ContextMenu`] renderer, and sends each command
//! the way the menu bar does (`menu_target::dispatch_menu_action`).
//!
//! Keyboard access follows Windows: tap Alt to open the first menu, press
//! Alt+letter to open the menu that starts with that letter, then ←/→ move
//! between menus, ↑/↓ between items, Return chooses and Esc closes.
//!
//! The strip is on wherever the Lulo menu bar is not: on Windows, unless
//! `LULO_MENU_BAR` says the phase 3 bar is running. `RMAC_IN_WINDOW_MENUS=1`
//! turns it on elsewhere, for development.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::OnceLock;

use gpui::{
    canvas, div, point, prelude::FluentBuilder as _, px, AnyWindowHandle, App, AppContext as _,
    Bounds, Context, DispatchPhase, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, MouseMoveEvent, ParentElement as _, Pixels, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window,
    WindowId,
};
use rmac_app_menu::{CheckState, Item, Menu};

use crate::{mac, text_px, ContextMenu, ContextMenuState, DismissMenu, MenuCheck};

/// The strip's height: the Mac menu bar's 24 pt.
pub const MENU_STRIP_HEIGHT: f32 = 24.0;

gpui::actions!(
    rmac_ui,
    [
        /// Alt tapped on its own: open the first menu, or close an open one.
        ToggleMenuStrip
    ]
);

/// A menu command chosen in the strip, sent like a menu-bar activation.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = rmac_ui, no_json)]
struct RunMenuStripCommand {
    name: SharedString,
}

/// Whether windows draw their own menus (see the module documentation).
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var_os("LULO_MENU_BAR").is_some() {
            return false;
        }
        cfg!(windows) || std::env::var_os("RMAC_IN_WINDOW_MENUS").is_some_and(|value| value == "1")
    })
}

/// The height app windows add for the strip: [`MENU_STRIP_HEIGHT`] once the
/// app's menus are installed with the strip on, otherwise nothing.
pub fn height(cx: &App) -> f32 {
    if cx.has_global::<MenuStrips>() {
        MENU_STRIP_HEIGHT
    } else {
        0.0
    }
}

struct MenuStrips {
    app_id: &'static str,
    strips: Vec<(AnyWindowHandle, WeakEntity<MenuStrip>)>,
}

impl gpui::Global for MenuStrips {}

thread_local! {
    /// App windows (those that track their state) not yet given a strip.
    static APP_WINDOWS: RefCell<HashSet<WindowId>> = RefCell::new(HashSet::new());
}

/// Mark `window` as an app window, so its root shows the strip. Panels such
/// as About do not call this and get none.
pub(crate) fn register_window(window: &Window) {
    if enabled() {
        APP_WINDOWS.with(|windows| {
            windows
                .borrow_mut()
                .insert(window.window_handle().window_id())
        });
    }
}

/// Give every app window of `app_id` a menu strip, when it is on.
pub(crate) fn install(app_id: &'static str, cx: &mut App) {
    if !enabled() {
        return;
    }
    cx.set_global(MenuStrips {
        app_id,
        strips: Vec::new(),
    });
    cx.set_global(crate::RootHeader(Rc::new(
        |window: &mut Window, cx: &mut App| {
            let id = window.window_handle().window_id();
            if !APP_WINDOWS.with(|windows| windows.borrow_mut().remove(&id)) {
                return None;
            }
            let strip = cx.new(MenuStrip::new);
            let handle = window.window_handle();
            let strips = &mut cx.global_mut::<MenuStrips>().strips;
            strips.retain(|(_, strip)| strip.upgrade().is_some());
            strips.push((handle, strip.downgrade()));
            Some(strip.into())
        },
    )));
    cx.bind_keys([KeyBinding::new("alt", ToggleMenuStrip, None)]);
    cx.on_action(|_: &ToggleMenuStrip, cx| {
        let Some(window) = cx.active_window() else {
            return;
        };
        with_strip(window, cx, |strip, window, cx| strip.toggle(window, cx));
    });
    cx.on_action(
        |command: &RunMenuStripCommand, cx| match cx.build_action(&command.name, None) {
            Ok(action) => crate::menu_target::dispatch_menu_action(action, cx),
            Err(error) => eprintln!("ignored unavailable menu command: {error}"),
        },
    );
    // Alt+letter opens the menu with that initial, as in Windows apps,
    // unless the app binds that chord itself.
    cx.observe_keystrokes(|event, window, cx| {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if event.action.is_some()
            || !modifiers.alt
            || modifiers.control
            || modifiers.platform
            || modifiers.shift
            || keystroke.key.chars().count() != 1
            || !keystroke.key.chars().all(|c| c.is_ascii_alphabetic())
        {
            return;
        }
        let letter = keystroke.key.to_ascii_lowercase();
        let handle = window.window_handle();
        let Some(strip) = strip_for(handle, cx) else {
            return;
        };
        strip.update(cx, |strip, cx| strip.open_initial(&letter, window, cx));
    })
    .detach();
    // A window is the only way back to a Windows app without the menu bar,
    // so closing the last one quits, as Calculator does on the Mac.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

fn strip_for(window: AnyWindowHandle, cx: &App) -> Option<gpui::Entity<MenuStrip>> {
    cx.try_global::<MenuStrips>()?
        .strips
        .iter()
        .find(|(handle, _)| *handle == window)
        .and_then(|(_, strip)| strip.upgrade())
}

fn with_strip(
    window: AnyWindowHandle,
    cx: &mut App,
    f: impl FnOnce(&mut MenuStrip, &mut Window, &mut Context<MenuStrip>),
) {
    let Some(strip) = strip_for(window, cx) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        strip.update(cx, |strip, cx| f(strip, window, cx));
    });
}

/// The menus as the strip shows them: the app menu under the app's name
/// with the standard Hide and Quit, the app's own menus, then Window (with
/// Minimize) and Help. The menu bar builds the same shape on Linux.
fn strip_menus(app_name: &str, menus: Vec<Menu>) -> Vec<Menu> {
    let mut exported = menus;
    let mut application = rmac_app_menu::take_application_items(&mut exported);
    let window_items = rmac_app_menu::take_window_items(&mut exported);
    let help_items = rmac_app_menu::take_help_items(&mut exported);

    let about = application
        .iter()
        .position(|item| item.action == rmac_app_menu::ABOUT_ACTION)
        .map(|index| application.remove(index));
    let keep_windows = application
        .iter()
        .position(|item| item.label == "Quit and Keep Windows")
        .map(|index| application.remove(index));
    let mut items = Vec::new();
    if about.is_some() {
        items.push(Item::new(
            format!("About {app_name}"),
            rmac_app_menu::ABOUT_ACTION,
            "",
        ));
    }
    if let Some(first) = application.first_mut() {
        first.separator_before = !items.is_empty();
    }
    items.extend(application);
    items.push(Item::new(format!("Hide {app_name}"), "rmac_ui::HideApplication", "⌘H").separated());
    items.push(Item::new(format!("Quit {app_name}"), "rmac_ui::QuitApplication", "⌘Q").separated());
    items.extend(keep_windows);

    let mut strip = vec![Menu {
        label: app_name.to_owned(),
        items,
    }];
    strip.extend(exported);
    let mut window = vec![Item::new("Minimize", "rmac_ui::MinimizeWindow", "⌘M")];
    if let Some(first) = window_items.first() {
        let mut first = first.clone();
        first.separator_before = true;
        window.push(first);
        window.extend(window_items.into_iter().skip(1));
    }
    strip.push(Menu {
        label: rmac_app_menu::WINDOW_MENU.to_owned(),
        items: window,
    });
    if !help_items.is_empty() {
        strip.push(Menu {
            label: "Help".to_owned(),
            items: help_items,
        });
    }
    strip
}

/// The menu whose title starts with `letter` (lower case), after `after`.
fn menu_for_initial(titles: &[SharedString], letter: &str, after: Option<usize>) -> Option<usize> {
    let count = titles.len();
    let start = after.map_or(0, |index| index + 1);
    (0..count)
        .map(|offset| (start + offset) % count)
        .find(|&index| {
            titles[index]
                .chars()
                .next()
                .is_some_and(|c| c.to_lowercase().to_string() == letter)
        })
}

fn context_menu(position: gpui::Point<Pixels>, items: &[Item]) -> ContextMenu {
    let mut menu = ContextMenu::new(position);
    for (index, item) in items.iter().enumerate() {
        if item.separator_before && index > 0 {
            menu = menu.separator();
        }
        menu = if item.is_submenu() {
            if item.enabled {
                menu.submenu(item.label.clone(), context_menu(position, &item.children))
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
                Box::new(RunMenuStripCommand {
                    name: item.action.clone().into(),
                }),
                item.enabled,
                check,
            )
        };
    }
    menu
}

struct OpenMenu {
    index: usize,
    items: Vec<Item>,
    state: ContextMenuState,
    return_focus: FocusHandle,
    _closed: Subscription,
}

/// One window's strip.
pub(crate) struct MenuStrip {
    app_name: SharedString,
    titles: Vec<SharedString>,
    focus: FocusHandle,
    open: Option<OpenMenu>,
    title_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
}

impl MenuStrip {
    fn new(cx: &mut Context<Self>) -> Self {
        let app_id = cx
            .try_global::<MenuStrips>()
            .map(|strips| strips.app_id)
            .unwrap_or_default();
        let app_name: SharedString = rmac_apps::identity::window_title(app_id)
            .unwrap_or(app_id)
            .to_owned()
            .into();
        // The titles never change; only the items' state does, and that is
        // read fresh each time a menu opens.
        let titles = strip_menus(
            &app_name,
            rmac_app_menu::definition(app_id, cx.all_action_names()).unwrap_or_default(),
        )
        .into_iter()
        .map(|menu| SharedString::from(menu.label))
        .collect();
        Self {
            app_name,
            titles,
            focus: cx.focus_handle(),
            open: None,
            title_bounds: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_some() {
            self.close(true, window, cx);
        } else {
            self.open_menu(0, window, cx);
        }
    }

    fn open_initial(&mut self, letter: &str, window: &mut Window, cx: &mut Context<Self>) {
        let after = self.open.as_ref().map(|open| open.index);
        if let Some(index) = menu_for_initial(&self.titles, letter, after) {
            self.open_menu(index, window, cx);
        }
    }

    fn open_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let menus = strip_menus(&self.app_name, crate::app_menu::current_menus(cx));
        let Some(menu) = menus.into_iter().nth(index) else {
            return;
        };
        // Commands go back to whatever had focus before the strip opened,
        // or to the content the window registered for menu commands.
        let return_focus = match self.open.take() {
            Some(open) => open.return_focus,
            None => window
                .focused(cx)
                .filter(|focused| !self.focus.contains(focused, window))
                .or_else(|| {
                    crate::menu_target::target(cx)
                        .filter(|(handle, _)| *handle == window.window_handle())
                        .and_then(|(_, focus)| focus)
                })
                .unwrap_or_else(|| self.focus.clone()),
        };
        let anchor = self.title_bounds.borrow().get(index).copied();
        let position = anchor.map_or(point(px(0.0), px(MENU_STRIP_HEIGHT)), |bounds| {
            point(bounds.left(), bounds.bottom())
        });
        let state = ContextMenuState::open(position, &return_focus, window, cx);
        // Choosing a command moves focus back out of the menu: close then.
        let menu_focus = state.menu_focus().clone();
        let watched = menu_focus.clone();
        let closed = cx.on_focus_out(&menu_focus, window, move |strip, _, window, cx| {
            if strip
                .open
                .as_ref()
                .is_some_and(|open| open.state.menu_focus() == &watched)
            {
                strip.close(false, window, cx);
            }
        });
        self.open = Some(OpenMenu {
            index,
            items: menu.items,
            state,
            return_focus,
            _closed: closed,
        });
        cx.notify();
    }

    fn close(&mut self, restore_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.take() else {
            return;
        };
        if restore_focus {
            window.focus(&open.return_focus, cx);
        }
        cx.notify();
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.open.as_ref().map(|open| open.index) else {
            return;
        };
        let count = self.titles.len().max(1);
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        self.open_menu(next, window, cx);
    }
}

impl Render for MenuStrip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open_index = self.open.as_ref().map(|open| open.index);
        let titles = self.titles.iter().enumerate().map(|(index, title)| {
            div()
                .id(("rmac-menu-strip-title", index))
                .role(Role::MenuItem)
                .aria_label(title.clone())
                .aria_expanded(open_index == Some(index))
                .h(px(MENU_STRIP_HEIGHT - 4.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(mac::radius_menu_item()))
                .when(index == 0, |title| title.font_weight(mac::BOLD))
                .when(open_index == Some(index), |title| title.bg(mac::hover()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |strip, _, window, cx| {
                        cx.stop_propagation();
                        if strip.open.as_ref().map(|open| open.index) == Some(index) {
                            strip.close(true, window, cx);
                        } else {
                            strip.open_menu(index, window, cx);
                        }
                    }),
                )
                .child(title.clone())
        });
        let bounds = self.title_bounds.clone();
        let row = div()
            .h_full()
            .flex()
            .items_center()
            .on_children_prepainted(move |children, _, _| *bounds.borrow_mut() = children)
            .children(titles);

        let open = self.open.as_ref().map(|open| {
            let viewport = window.viewport_size();
            // While a menu is open, moving onto another title opens that
            // one, as in the Mac menu bar. The menu's click-away layer
            // covers the strip, so this reads the pointer directly.
            let bounds = self.title_bounds.clone();
            let strip = cx.weak_entity();
            let current = open.index;
            let switcher = canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let hovered = bounds
                            .borrow()
                            .iter()
                            .position(|title| title.contains(&event.position));
                        if let Some(index) = hovered.filter(|&index| index != current) {
                            let _ =
                                strip.update(cx, |strip, cx| strip.open_menu(index, window, cx));
                        }
                    });
                },
            )
            .absolute()
            .size_0();
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(viewport.width)
                .h(viewport.height)
                .child(context_menu(open.state.position(), &open.items).render(&open.state))
                .child(switcher)
        });

        div()
            .id("rmac-menu-strip")
            .role(Role::MenuBar)
            .aria_label("Menu bar")
            .track_focus(&self.focus)
            .relative()
            .w_full()
            .h(px(MENU_STRIP_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px(px(6.0))
            .bg(mac::chrome())
            .border_b_1()
            .border_color(mac::separator())
            .text_size(text_px(13.0))
            .text_color(mac::text())
            .on_action(cx.listener(|strip, _: &DismissMenu, window, cx| {
                strip.close(true, window, cx);
            }))
            .on_key_down(cx.listener(|strip, event: &KeyDownEvent, window, cx| {
                if strip.open.is_none() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "left" => strip.step(false, window, cx),
                    "right" => strip.step(true, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(row)
            // The empty rest of the strip moves the window, like a title bar.
            .child(
                crate::title_bar_drag_region("rmac-menu-strip-drag")
                    .flex_1()
                    .h_full(),
            )
            .children(open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(label: &str, items: Vec<Item>) -> Menu {
        Menu {
            label: label.to_owned(),
            items,
        }
    }

    #[test]
    fn the_strip_adds_the_standard_app_window_menus() {
        let menus = vec![
            menu(
                rmac_app_menu::APPLICATION_MENU,
                vec![
                    Item::new("About", rmac_app_menu::ABOUT_ACTION, ""),
                    Item::new("Settings…", "notes::ShowSettings", "⌘,"),
                ],
            ),
            menu(
                "File",
                vec![Item::new("New Note", "notes::ComposeNote", "⌘N")],
            ),
            menu(
                rmac_app_menu::WINDOW_MENU,
                vec![Item::new("Notes", "notes::FocusMainWindow", "⌘0")],
            ),
        ];
        let strip = strip_menus("Notes", menus);
        let labels = strip
            .iter()
            .map(|menu| menu.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["Notes", "File", "Window"]);
        let app = strip[0]
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            app,
            ["About Notes", "Settings…", "Hide Notes", "Quit Notes"]
        );
        assert!(strip[0].items[1].separator_before);
        let window = &strip[2].items;
        assert_eq!(window[0].action, "rmac_ui::MinimizeWindow");
        assert_eq!(window[1].action, "notes::FocusMainWindow");
        assert!(window[1].separator_before);
        // Every command in the strip is distinct.
        let mut actions = strip
            .iter()
            .flat_map(Menu::leaves)
            .map(|(_, item)| item.action.clone())
            .collect::<Vec<_>>();
        let count = actions.len();
        actions.sort();
        actions.dedup();
        assert_eq!(actions.len(), count);
    }

    #[test]
    fn alt_letters_open_menus_by_initial_in_turn() {
        let titles = ["Notes", "File", "Edit", "Format", "Window"]
            .map(SharedString::from)
            .to_vec();
        assert_eq!(menu_for_initial(&titles, "f", None), Some(1));
        assert_eq!(menu_for_initial(&titles, "f", Some(1)), Some(3));
        assert_eq!(menu_for_initial(&titles, "f", Some(3)), Some(1));
        assert_eq!(menu_for_initial(&titles, "w", None), Some(4));
        assert_eq!(menu_for_initial(&titles, "z", None), None);
    }
}
