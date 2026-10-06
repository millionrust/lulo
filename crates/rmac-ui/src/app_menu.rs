//! The app side of the menu bar contract (`org.rmac.AppMenu1` and
//! `org.rmac.AppMenu2`, see `rmac-app-menu`).
//!
//! An app's menus are a static table in `rmac-app-menu`; this module adds
//! their live state, the way AppKit's `validateMenuItem` does:
//!
//! - The app states what only it knows with [`set_menu_checked`],
//!   [`set_menu_enabled`] and [`set_menu_label`]: which sort order is
//!   ticked, whether there is a note to pin. Calls that change nothing cost
//!   a map lookup, so a view may make them from `render`. A change is
//!   published once per frame at most and the menu bar is told with
//!   `LayoutChanged`; nothing is polled.
//! - The text-field Edit commands (`input::Undo`, `input::Copy`, …) are
//!   enabled only while a text field in the key window has focus, which
//!   GPUI's dispatch tree answers directly.
//! - When the menu bar opens a menu it asks for `Layout`, and the endpoint
//!   asks this module to validate again first, so focus changes since the
//!   last publish are reflected without the app announcing them.
//! - That same moment rebuilds File ▸ Open Recent ▸ for an app whose menu
//!   table has one (see `rmac_app_menu::recent`), from whichever documents
//!   `rmac-recent-documents` says it opened. Reading the store then is the
//!   whole invalidation story: nothing is polled, and an app that clears its
//!   Recents needs to publish nothing either.

// The menu bar, and so this module's endpoint, exists only on Linux. On
// Windows the same model feeds each window's menu strip (`menu_strip`).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use gpui::{App, Global};
use rmac_app_menu::{CheckState, Item, ItemState, Menu};

// App ▸ About <App>: the Mac-style About panel. Its name is
// `rmac_app_menu::ABOUT_ACTION`.
gpui::actions!(rmac, [ShowAboutPanel]);

struct MenuModel {
    app_id: &'static str,
    definition: Vec<Menu>,
    overrides: BTreeMap<String, ItemState>,
    dynamic_children: BTreeMap<String, Vec<Item>>,
    publisher: Option<rmac_app_menu::Publisher>,
    publish_scheduled: bool,
}

impl Global for MenuModel {}

/// Tick or untick a menu command, for example the current sort order.
pub fn set_menu_checked(action: &str, checked: bool, cx: &mut App) {
    update_item(action, cx, |state| {
        state.checked = Some(CheckState::from(checked))
    });
}

/// Show a command as a dash: the choice applies to part of the selection.
pub fn set_menu_mixed(action: &str, cx: &mut App) {
    update_item(action, cx, |state| state.checked = Some(CheckState::Mixed));
}

/// Grey out a command that does not apply now (nothing selected, nothing
/// to pin), or enable it again.
pub fn set_menu_enabled(action: &str, enabled: bool, cx: &mut App) {
    update_item(action, cx, |state| state.enabled = Some(enabled));
}

/// Rename a command for the current state: "Pin Note" / "Unpin Note".
pub fn set_menu_label(action: &str, label: &str, cx: &mut App) {
    update_item(action, cx, |state| {
        if state.label.as_deref() != Some(label) {
            state.label = Some(label.to_owned());
        }
    });
}

/// Replace a submenu's children with app-owned rows, such as a live list of
/// recently visited notes. The parent action must exist in the definition.
pub fn set_menu_children(action: &str, children: Vec<Item>, cx: &mut App) {
    let Some(model) = cx.try_global::<MenuModel>() else {
        return;
    };
    if model.dynamic_children.get(action) == Some(&children) {
        return;
    }
    cx.global_mut::<MenuModel>()
        .dynamic_children
        .insert(action.to_owned(), children);
    schedule_publish(cx);
}

fn update_item(action: &str, cx: &mut App, change: impl FnOnce(&mut ItemState)) {
    let Some(model) = cx.try_global::<MenuModel>() else {
        // This app publishes no menus.
        return;
    };
    let current = model.overrides.get(action).cloned().unwrap_or_default();
    let mut next = current.clone();
    change(&mut next);
    if next == current {
        return;
    }
    cx.global_mut::<MenuModel>()
        .overrides
        .insert(action.to_owned(), next);
    schedule_publish(cx);
}

/// Publish the current state after this frame's updates, once.
fn schedule_publish(cx: &mut App) {
    let model = cx.global_mut::<MenuModel>();
    if model.publish_scheduled || model.publisher.is_none() {
        // Not on the bus yet: the first publish carries every change.
        return;
    }
    model.publish_scheduled = true;
    cx.defer(|cx| {
        let menus = current_menus(cx);
        let model = cx.global_mut::<MenuModel>();
        model.publish_scheduled = false;
        let Some(publisher) = model.publisher.clone() else {
            return;
        };
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = publisher.publish(menus).await {
                    eprintln!("the menu bar was not told about changed menus: {error}");
                }
            })
            .detach();
    });
}

/// The menus with every item's state as of now.
pub(crate) fn current_menus(cx: &mut App) -> Vec<Menu> {
    let Some(model) = cx.try_global::<MenuModel>() else {
        return Vec::new();
    };
    let app_id = model.app_id;
    let definition = model.definition.clone();
    let overrides = model.overrides.clone();
    let dynamic_children = model.dynamic_children.clone();
    let text_actions = definition
        .iter()
        .flat_map(Menu::leaves)
        .map(|(_, item)| item.action.clone())
        .filter(|action| action.starts_with(rmac_app_menu::TEXT_FIELD_ACTION_PREFIX))
        .collect::<Vec<_>>();
    let available = available_in_key_window(&text_actions, cx);
    let mut menus = rmac_app_menu::apply_state(&definition, |item| {
        let mut state = overrides.get(&item.action).cloned().unwrap_or_default();
        if item
            .action
            .starts_with(rmac_app_menu::TEXT_FIELD_ACTION_PREFIX)
            && !available.contains(&item.action)
        {
            state.enabled = Some(false);
        }
        if item.action == rmac_app_menu::PASTE_MATCH_STYLE_ACTION
            && !available.contains("input::Paste")
        {
            state.enabled = Some(false);
        }
        Some(state)
    });
    fn replace_children(
        items: &mut [Item],
        replacements: &BTreeMap<String, Vec<Item>>,
        overrides: &BTreeMap<String, ItemState>,
    ) {
        for item in items {
            if let Some(children) = replacements.get(&item.action) {
                item.children = children.clone();
                item.enabled = overrides
                    .get(&item.action)
                    .and_then(|state| state.enabled)
                    .unwrap_or_else(|| item.children.iter().any(|child| child.enabled));
            } else {
                replace_children(&mut item.children, replacements, overrides);
            }
        }
    }
    for menu in &mut menus {
        replace_children(&mut menu.items, &dynamic_children, &overrides);
    }
    // File ▸ Open Recent ▸ (TE-02, PREV-08/PREV-15): built fresh every time
    // a menu is about to open, from whichever documents `app_id` itself
    // recorded — nothing to poll, and nothing to announce when the store
    // changes. A no-op, and no store read at all, for an app whose menu
    // table has no such submenu.
    if let Some(prefix) = rmac_app_menu::recent_documents_prefix(app_id) {
        rmac_app_menu::recent::refresh(&mut menus, prefix, || {
            rmac_recent_documents::Store::from_environment()
                .and_then(|store| store.load_for_app(app_id))
                .unwrap_or_default()
        });
    }
    menus
}

/// Which of `actions` a handler in the key window's focused element (or
/// the content registered with [`crate::register_menu_target`]) answers.
fn available_in_key_window(actions: &[String], cx: &mut App) -> BTreeSet<String> {
    if actions.is_empty() {
        return BTreeSet::new();
    }
    let Some((window, focus)) = crate::menu_target::target(cx) else {
        return BTreeSet::new();
    };
    let built = actions
        .iter()
        .filter_map(|name| {
            cx.build_action(name, None)
                .ok()
                .map(|action| (name.clone(), action))
        })
        .collect::<Vec<_>>();
    window
        .update(cx, |_, window, cx| {
            built
                .iter()
                .filter(|(_, action)| {
                    window.is_action_available(action.as_ref(), cx)
                        || focus.as_ref().is_some_and(|focus| {
                            window.is_action_available_in(action.as_ref(), focus)
                        })
                })
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) type OpenWindowRequest = Box<dyn Fn(Vec<String>, &mut App)>;

/// Publish this app's menus, answer the menu bar's validation requests and
/// route its activations into the app (see [`crate::register_menu_target`]).
pub(crate) fn install(app_id: &'static str, open_window: Option<OpenWindowRequest>, cx: &mut App) {
    // Windows has no Lulo menu bar yet: a later launch reaches this process
    // over a named pipe instead of D-Bus (ADR 0023).
    #[cfg(windows)]
    let open_window: Option<OpenWindowRequest> = {
        if let Some(open_window) = open_window {
            crate::instance_windows::serve(app_id, open_window, cx);
        }
        None
    };
    let Some(menus) = rmac_app_menu::definition(app_id, cx.all_action_names()) else {
        return;
    };
    cx.on_action(move |_: &ShowAboutPanel, cx| crate::about::show(app_id, cx));
    cx.set_global(MenuModel {
        app_id,
        definition: menus.clone(),
        overrides: BTreeMap::new(),
        dynamic_children: BTreeMap::new(),
        publisher: None,
        publish_scheduled: false,
    });
    // Without the Lulo menu bar, every window shows these menus itself.
    crate::menu_strip::install(app_id, cx);
    #[cfg(target_os = "linux")]
    {
        let (activation_tx, activation_rx) = rmac_app_menu::activation_channel();
        let (validation_tx, validation_rx) = rmac_app_menu::validation_channel();
        let windows = open_window.map(|open_window| {
            let (window_tx, window_rx) = rmac_app_menu::window_request_channel();
            cx.spawn(async move |cx| {
                while let Ok(arguments) = window_rx.recv().await {
                    cx.update(|cx| open_window(arguments, cx));
                }
            })
            .detach();
            window_tx
        });
        cx.spawn(async move |cx| {
            while let Ok(reply) = validation_rx.recv().await {
                let menus = cx.update(current_menus);
                // A reader that gave up waiting has dropped its end; it
                // was already served the last published state.
                if reply.try_send(menus).is_err() {
                    eprintln!("{app_id}: a menu validation arrived after its reader gave up");
                }
            }
        })
        .detach();
        let options = rmac_app_menu::EndpointOptions {
            windows,
            validation: Some(validation_tx),
        };
        let served = cx.background_executor().spawn(async move {
            rmac_app_menu::serve_menus(app_id, menus, activation_tx, options).await
        });
        cx.spawn(async move |cx| match served.await {
            // The name is owned and the endpoint lives until the process
            // exits; publish whatever state the app set while it started.
            Ok(publisher) => cx.update(|cx| {
                cx.global_mut::<MenuModel>().publisher = Some(publisher);
                schedule_publish(cx);
            }),
            Err(error) => {
                eprintln!("{app_id}: the menu bar cannot show this app's menus: {error}")
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            while let Ok(action_name) = activation_rx.recv().await {
                cx.update(|cx| match cx.build_action(&action_name, None) {
                    Ok(action) => crate::menu_target::dispatch_menu_action(action, cx),
                    Err(error) => eprintln!("ignored unavailable {app_id} menu action: {error}"),
                });
            }
        })
        .detach();
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (menus, open_window);
}
