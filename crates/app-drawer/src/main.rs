//! rmac App Drawer — a Launchpad / App-Library-style app grid.
//!
//! Uses `rmac-apps` to discover macOS bundles or Linux desktop entries, resolves
//! real icons, and renders a searchable grid. Clicking an app launches it.
//!
//! Beyond the basic grid this adds App-Library-style ergonomics:
//!   * keyboard navigation — arrow keys move a selection cursor over the grid
//!     (or list), Return launches the selected app, Escape clears the search;
//!   * a grid/list view toggle in the toolbar;
//!   * a category derived per app (from its folder + a name keyword map) and a
//!     category filter bar.

#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::time::Duration;

use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AnyWindowHandle, App as GpuiApp,
    AppContext as _, Application, BorrowAppContext as _, Context, Div, Entity, FocusHandle, Global,
    InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled,
    WeakEntity, Window,
};
use gpui_component::{Root, StyledExt as _};
use rmac_app_drawer::{run_mode, RunMode};
use rmac_ui::{mac, EmptyState, InputState, SearchField};

const TILE_W: f32 = 116.0;
const ICON: f32 = 60.0;
const ROW_ICON: f32 = 32.0;

actions!(
    app_drawer,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        Launch,
        ClearSearch,
        OpenApp,
        RevealInFinder
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Grid,
    List,
}

/// App-Library-style buckets. Derived per app from its install folder and a
/// keyword map over its name (see [`categorize`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Productivity,
    Internet,
    Media,
    Developer,
    Utilities,
    Games,
    System,
    Other,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Category::Productivity => "Productivity",
            Category::Internet => "Internet",
            Category::Media => "Media",
            Category::Developer => "Developer",
            Category::Utilities => "Utilities",
            Category::Games => "Games",
            Category::System => "System",
            Category::Other => "Other",
        }
    }

    /// Display order for the filter bar.
    const ORDER: [Category; 8] = [
        Category::Productivity,
        Category::Internet,
        Category::Media,
        Category::Developer,
        Category::Utilities,
        Category::Games,
        Category::System,
        Category::Other,
    ];
}

#[derive(Clone)]
struct App {
    id: String,
    name: SharedString,
    generic_name: Option<String>,
    keywords: Vec<String>,
    path: PathBuf,
    icon: Option<PathBuf>,
    category: Category,
    source_categories: Vec<String>,
    mime_types: Vec<String>,
    search_text: String,
    launch: rmac_apps::LaunchSpec,
    actions: Vec<rmac_apps::DesktopAction>,
}

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = app_drawer, no_json)]
struct LaunchDesktopAction {
    launch: rmac_apps::LaunchSpec,
}

struct AppDrawer {
    service_token: Option<u64>,
    apps: Vec<App>,
    query: Entity<InputState>,
    focus: FocusHandle,
    view: ViewMode,
    /// `None` == "All".
    filter: Option<Category>,
    /// Cursor into the currently-visible (filtered) list.
    selected: usize,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<Point<Pixels>>,
    /// Columns in the grid as last laid out — used for up/down navigation.
    cols: usize,
    catalog_error: Option<SharedString>,
    action_error: Option<SharedString>,
    launching: bool,
    _catalog_watcher: Option<rmac_apps::CatalogWatcher>,
}

#[derive(Clone)]
struct ActiveDrawer {
    token: u64,
    view: WeakEntity<AppDrawer>,
    window: AnyWindowHandle,
}

struct AppDrawerService {
    active: Option<ActiveDrawer>,
    next_token: u64,
}

impl Global for AppDrawerService {}

impl AppDrawer {
    fn new(service_token: Option<u64>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (apps, mut catalog_error) = scan_apps();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        if let Some(token) = service_token {
            cx.on_release(move |_, cx| {
                if cx.has_global::<AppDrawerService>() {
                    cx.update_global::<AppDrawerService, _>(|service, _| {
                        if service
                            .active
                            .as_ref()
                            .is_some_and(|active| active.token == token)
                        {
                            service.active = None;
                        }
                    });
                }
            })
            .detach();
        }

        // Typing in the search field re-anchors the cursor to the first match
        // and is the redraw trigger for live filtering.
        cx.observe(&query, |this: &mut AppDrawer, _, cx| {
            this.selected = 0;
            // If the active category filter no longer has any matches under the
            // new search, drop back to "All" so we never show an empty view.
            if let Some(c) = this.filter {
                if !this.present_categories(cx).contains(&c) {
                    this.filter = None;
                }
            }
            cx.notify();
        })
        .detach();

        let focus = cx.focus_handle();
        focus.focus(window);

        // Extract macOS bundle icons off the main thread, then fill them in.
        #[cfg(target_os = "macos")]
        {
            let snapshot: Vec<(String, PathBuf)> = apps
                .iter()
                .map(|a| (a.name.to_string(), a.path.clone()))
                .collect();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                let icons = cx
                    .background_executor()
                    .spawn(async move {
                        let cache = cache_dir();
                        snapshot
                            .into_iter()
                            .map(|(name, path)| {
                                let icon = extract_icon(&name, &path, &cache);
                                (path, icon)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = this.update(cx, |this: &mut AppDrawer, cx| {
                    for (path, icon) in icons {
                        if let Some(a) = this.apps.iter_mut().find(|app| app.path == path) {
                            a.icon = icon;
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        // Native filesystem notifications wake this task only when an app
        // entry changes. The capacity-one channel coalesces event bursts before
        // discovery and icon/category work runs off the UI thread.
        let (catalog_events, catalog_event_rx) = async_channel::bounded(1);
        let catalog_watcher = match rmac_apps::watch_catalog(move || {
            signal_catalog_change(&catalog_events);
        }) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                if catalog_error.is_none() {
                    catalog_error = Some(
                        format!("Applications loaded, but live updates are unavailable: {error}")
                            .into(),
                    );
                }
                None
            }
        };
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while catalog_event_rx.recv().await.is_ok() {
                for _ in 0..10 {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    if catalog_event_rx.try_recv().is_err() {
                        break;
                    }
                }
                while catalog_event_rx.try_recv().is_ok() {}

                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let (mut apps, error) = scan_apps();
                        #[cfg(target_os = "macos")]
                        hydrate_icons(&mut apps);
                        (apps, error)
                    })
                    .await;
                if this
                    .update(cx, |this: &mut AppDrawer, cx| {
                        this.replace_catalog(result.0, result.1, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self {
            service_token,
            apps,
            query,
            focus,
            view: ViewMode::Grid,
            filter: None,
            selected: 0,
            menu_at: None,
            cols: 6,
            catalog_error,
            action_error: None,
            launching: false,
            _catalog_watcher: catalog_watcher,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(token) = self.service_token {
            if cx.has_global::<AppDrawerService>() {
                cx.update_global::<AppDrawerService, _>(|service, _| {
                    if service
                        .active
                        .as_ref()
                        .is_some_and(|active| active.token == token)
                    {
                        service.active = None;
                    }
                });
            }
        }
        window.remove_window();
    }

    fn replace_catalog(
        &mut self,
        apps: Vec<App>,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(error) = error {
            // A transient read failure should not erase a usable catalog.
            self.catalog_error = Some(error);
            cx.notify();
            return;
        }

        let previous_visible = self.visible_indices(cx);
        let selected_path = previous_visible
            .get(self.selected.min(previous_visible.len().saturating_sub(1)))
            .and_then(|index| self.apps.get(*index))
            .map(|app| app.path.clone());
        self.apps = apps;
        self.catalog_error = None;
        self.menu_at = None;
        if self
            .filter
            .is_some_and(|filter| !self.present_categories(cx).contains(&filter))
        {
            self.filter = None;
        }
        let visible = self.visible_indices(cx);
        self.selected = selected_path
            .and_then(|path| {
                visible
                    .iter()
                    .position(|index| self.apps[*index].path == path)
            })
            .unwrap_or(0);
        cx.notify();
    }

    /// Indices into `self.apps` that pass the current category + search filter.
    fn visible_indices(&self, cx: &gpui::App) -> Vec<usize> {
        let q = self.query.read(cx).value().trim().to_lowercase();
        self.apps
            .iter()
            .enumerate()
            .filter(|(_, a)| self.filter.is_none_or(|c| a.category == c))
            .filter(|(_, a)| q.is_empty() || a.search_text.contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    /// The set of categories actually present after the *search* filter — used
    /// to build the category bar so we never show an empty bucket.
    fn present_categories(&self, cx: &gpui::App) -> Vec<Category> {
        let q = self.query.read(cx).value().trim().to_lowercase();
        Category::ORDER
            .into_iter()
            .filter(|c| {
                self.apps
                    .iter()
                    .any(|a| a.category == *c && (q.is_empty() || a.search_text.contains(&q)))
            })
            .collect()
    }

    fn move_by(&mut self, dx: isize, dy: isize, cx: &mut Context<Self>) {
        let count = self.visible_indices(cx).len();
        if count == 0 {
            return;
        }
        let cols = if self.view == ViewMode::List {
            1
        } else {
            self.cols.max(1)
        } as isize;
        let cur = self.selected.min(count - 1) as isize;
        let step = dx + dy * cols;
        let next = (cur + step).clamp(0, count as isize - 1);
        self.selected = next as usize;
        cx.notify();
    }

    fn launch_selected(&mut self, cx: &mut Context<Self>) {
        let vis = self.visible_indices(cx);
        if let Some(&idx) = vis.get(self.selected.min(vis.len().saturating_sub(1))) {
            let launch = self.apps[idx].launch.clone();
            self.launch_application(launch, cx);
        }
    }

    fn selected_app(&self, cx: &gpui::App) -> Option<rmac_apps::Application> {
        let vis = self.visible_indices(cx);
        let &idx = vis.get(self.selected.min(vis.len().saturating_sub(1)))?;
        self.apps.get(idx).map(|app| rmac_apps::Application {
            id: app.id.clone(),
            name: app.name.to_string(),
            generic_name: app.generic_name.clone(),
            keywords: app.keywords.clone(),
            source: app.path.clone(),
            icon: app.icon.clone(),
            categories: app.source_categories.clone(),
            mime_types: app.mime_types.clone(),
            launch: app.launch.clone(),
            actions: app.actions.clone(),
        })
    }

    fn launch_application(&mut self, launch: rmac_apps::LaunchSpec, cx: &mut Context<Self>) {
        if self.launching {
            return;
        }
        self.launching = true;
        self.action_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::launch(launch).await;
            let _ = this.update(cx, |this, cx| {
                this.launching = false;
                this.action_error = result
                    .err()
                    .map(|error| format!("Could not open application: {error}").into());
                cx.notify();
            });
        })
        .detach();
    }

    /// Reveal the selected app in the real Finder. GPUI can't initiate a native
    /// drag out to the Dock/desktop, so this is the honest bridge: it takes you
    /// to the app in Finder, where it can be dragged onto the Dock.
    fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(application) = self.selected_app(cx) {
            cx.spawn(async move |this, cx| {
                let result = rmac_app_launch::reveal_application(application).await;
                let _ = this.update(cx, |this, cx| {
                    this.catalog_error = result
                        .err()
                        .map(|error| format!("Could not show application: {error}").into());
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(application) = self.selected_app(cx) {
            self.launch_application(application.launch, cx);
        }
    }

    /// The right-click menu shared by grid tiles and list rows. Desktop-entry
    /// actions keep their declared order and exact localized labels.
    fn app_menu(&self, pos: Point<Pixels>, cx: &gpui::App) -> rmac_ui::ContextMenu {
        let mut menu = rmac_ui::ContextMenu::new(pos).command_item(
            "Open",
            rmac_ui::shortcuts::ENTER,
            Box::new(OpenApp),
        );
        if let Some(application) = self.selected_app(cx) {
            for action in application.actions {
                menu = menu.item(
                    action.name,
                    Box::new(LaunchDesktopAction {
                        launch: action.launch,
                    }),
                );
            }
        }
        menu.separator()
            .item("Show in Folder", Box::new(RevealInFinder))
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |st, cx| st.set_value("", window, cx));
        self.selected = 0;
        self.focus.focus(window);
        cx.notify();
    }

    fn set_filter(&mut self, filter: Option<Category>, cx: &mut Context<Self>) {
        self.filter = filter;
        self.selected = 0;
        cx.notify();
    }

    fn icon_element(&self, app: &App, size: f32) -> gpui::AnyElement {
        match &app.icon {
            Some(p) => img(p.clone()).w(px(size)).h(px(size)).into_any_element(),
            None => {
                // Placeholder squircle with the app initial until the icon loads.
                let initial = app
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                div()
                    .w(px(size))
                    .h(px(size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(size * 0.23))
                    .bg(mac::control_fill())
                    .text_color(mac::text_secondary())
                    .text_size(rmac_ui::text_px(size * 0.43))
                    .child(initial)
                    .into_any_element()
            }
        }
    }

    /// `pos` is the position in the currently-visible list (what `selected`
    /// tracks); `path` is used for the launch path lookup.
    fn tile(&self, app: &App, pos: usize, selected: bool, cx: &Context<Self>) -> impl IntoElement {
        let launch = app.launch.clone();
        div()
            .id(SharedString::from(format!("app-{}", app.path.display())))
            .w(px(TILE_W))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .px_1()
            .py_2()
            .rounded(px(10.0))
            .when(selected, |d: Stateful<Div>| {
                d.bg(mac::accent_subtle())
                    .border_1()
                    .border_color(mac::accent_border())
            })
            .when(!selected, |d: Stateful<Div>| {
                d.border_1().border_color(gpui::transparent_black())
            })
            .hover(|h| h.bg(mac::hover()))
            .child(self.icon_element(app, ICON))
            .child(
                div()
                    .max_w(px(TILE_W - 8.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text())
                    .text_center()
                    .truncate()
                    .child(app.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = pos;
                this.launch_application(launch.clone(), cx);
            }))
            // Right-click selects this tile so the menu acts on it.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.selected = pos;
                    this.menu_at = Some(ev.position);
                    cx.notify();
                }),
            )
    }

    /// `pos` is the position in the currently-visible list (what `selected`
    /// tracks); `path` is used for the launch path lookup.
    fn row(&self, app: &App, pos: usize, selected: bool, cx: &Context<Self>) -> impl IntoElement {
        let launch = app.launch.clone();
        div()
            .id(SharedString::from(format!("row-{}", app.path.display())))
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .px_3()
            .py_1p5()
            .rounded(px(8.0))
            .when(selected, |d: Stateful<Div>| d.bg(mac::accent_subtle()))
            .when(!selected, |d: Stateful<Div>| {
                d.hover(|h| h.bg(mac::hover()))
            })
            .child(self.icon_element(app, ROW_ICON))
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text())
                    .truncate()
                    .child(app.name.clone()),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::text_secondary())
                    .child(app.category.label()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = pos;
                this.launch_application(launch.clone(), cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.selected = pos;
                    this.menu_at = Some(ev.position);
                    cx.notify();
                }),
            )
    }

    fn view_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let seg = |id: &'static str, glyph: &'static str, mode: ViewMode, active: bool| {
            div()
                .id(id)
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(active, |d: Stateful<Div>| d.bg(mac::raised()))
                .child(
                    svg()
                        .path(glyph)
                        .w(px(15.0))
                        .h(px(15.0))
                        .text_color(if active {
                            mac::text()
                        } else {
                            mac::text_secondary()
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.view = mode;
                    cx.notify();
                }))
        };
        div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(mac::control_fill())
            .child(seg(
                "v-grid",
                "icons/layout-dashboard.svg",
                ViewMode::Grid,
                self.view == ViewMode::Grid,
            ))
            .child(seg(
                "v-list",
                "icons/menu.svg",
                ViewMode::List,
                self.view == ViewMode::List,
            ))
    }

    fn category_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let present = self.present_categories(cx);
        let pill =
            |id: SharedString, label: SharedString, active: bool, target: Option<Category>| {
                div()
                    .id(id)
                    .px_3()
                    .py_1()
                    .rounded(px(13.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .when(active, |d: Stateful<Div>| {
                        d.bg(mac::accent()).text_color(mac::on_accent())
                    })
                    .when(!active, |d: Stateful<Div>| {
                        d.bg(mac::control_fill())
                            .text_color(mac::text())
                            .hover(|h| h.bg(mac::control_fill_hover()))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filter(target, cx)))
            };

        let mut bar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .justify_center()
            .px_8()
            .pb_2()
            .child(pill(
                "cat-all".into(),
                "All".into(),
                self.filter.is_none(),
                None,
            ));
        for c in present {
            bar = bar.child(pill(
                SharedString::from(format!("cat-{}", c.label())),
                c.label().into(),
                self.filter == Some(c),
                Some(c),
            ));
        }
        bar
    }
}

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Recompute the live grid column count from the viewport so up/down
        // navigation matches what the user sees.
        let vw = f32::from(window.viewport_size().width);
        let usable = (vw - 64.0).max(TILE_W);
        self.cols = ((usable / (TILE_W + 8.0)).floor() as usize).max(1);

        let vis = self.visible_indices(cx);
        let sel = self.selected.min(vis.len().saturating_sub(1));
        let notice = if self.launching {
            Some((SharedString::from("Opening application…"), false))
        } else {
            self.action_error
                .clone()
                .or_else(|| self.catalog_error.clone())
                .map(|message| (message, true))
        };
        let context_menu = self.menu_at.map(|pos| self.app_menu(pos, cx));

        let body: gpui::AnyElement = if vis.is_empty() {
            let empty = if self.apps.is_empty() {
                EmptyState::new("No applications found").message(
                    "Install an application or add a visible desktop entry to an XDG application directory",
                )
            } else {
                EmptyState::new("No matching applications")
                    .message("Try another name, keyword, category, or application action")
            };
            div()
                .min_h(px(320.0))
                .w_full()
                .flex()
                .items_center()
                .child(empty)
                .into_any_element()
        } else if self.view == ViewMode::Grid {
            let tiles = vis
                .iter()
                .enumerate()
                .map(|(pos, &idx)| self.tile(&self.apps[idx], pos, pos == sel, cx))
                .collect::<Vec<_>>();
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .justify_center()
                .children(tiles)
                .into_any_element()
        } else {
            let rows = vis
                .iter()
                .enumerate()
                .map(|(pos, &idx)| self.row(&self.apps[idx], pos, pos == sel, cx))
                .collect::<Vec<_>>();
            div()
                .v_flex()
                .gap_0p5()
                .w_full()
                .max_w(px(640.0))
                .mx_auto()
                .children(rows)
                .into_any_element()
        };

        div()
            .track_focus(&self.focus)
            .key_context("AppDrawer")
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.move_by(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.move_by(1, 0, cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_by(0, -1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_by(0, 1, cx)))
            .on_action(cx.listener(|this, _: &Launch, _, cx| this.launch_selected(cx)))
            .on_action(cx.listener(|this, _: &OpenApp, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, action: &LaunchDesktopAction, _, cx| {
                this.launch_application(action.launch.clone(), cx)
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| this.reveal_selected(cx)))
            .on_action(
                cx.listener(|this, _: &ClearSearch, window, cx| this.clear_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| this.dismiss(window, cx)),
            )
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("Applications"))
            .when_some(notice, |drawer, (message, is_error)| {
                drawer.child(
                    div()
                        .id("app-drawer-notice")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(if is_error {
                            mac::error_background()
                        } else {
                            mac::control_fill()
                        })
                        .border_b_1()
                        .border_color(if is_error {
                            mac::error_border()
                        } else {
                            mac::separator()
                        })
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(if is_error {
                            mac::danger()
                        } else {
                            mac::text_secondary()
                        })
                        .when(is_error, |notice| notice.cursor_pointer())
                        .child(div().flex_1().child(message))
                        .when(is_error, |notice| notice.child("Dismiss"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.launching {
                                this.catalog_error = None;
                                this.action_error = None;
                                cx.notify();
                            }
                        })),
                )
            })
            .child(
                // toolbar: centered search with the view toggle pinned right.
                div()
                    .flex()
                    .items_center()
                    .px_8()
                    .py_4()
                    .child(div().w(px(34.0)))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .child(div().w(px(280.0)).child(SearchField::new(&self.query))),
                    )
                    .child(self.view_toggle(cx)),
            )
            .child(self.category_bar(cx))
            .child(
                div()
                    .id("grid-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_8()
                    .pb_8()
                    .child(body),
            )
            .when_some(context_menu, |el: Div, menu| el.child(menu.render()))
    }
}

// ---- app discovery & icon extraction ----

fn scan_apps() -> (Vec<App>, Option<SharedString>) {
    let catalog = match rmac_apps::discover() {
        Ok(catalog) => catalog,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("Could not load applications: {error}").into()),
            );
        }
    };

    #[cfg(target_os = "macos")]
    let categories = {
        let pairs = catalog
            .iter()
            .map(|application| (application.name.clone(), application.source.clone()))
            .collect::<Vec<_>>();
        parallel_categorize(&pairs)
    };
    #[cfg(not(target_os = "macos"))]
    let categories = catalog
        .iter()
        .map(|application| categorize_desktop(&application.categories))
        .collect::<Vec<_>>();

    let apps = catalog
        .into_iter()
        .zip(categories)
        .map(|(application, category)| App {
            search_text: format!(
                "{}\n{}",
                application.searchable_text(),
                category.label().to_lowercase()
            ),
            id: application.id,
            name: application.name.into(),
            generic_name: application.generic_name,
            keywords: application.keywords,
            path: application.source,
            icon: application.icon,
            category,
            source_categories: application.categories,
            mime_types: application.mime_types,
            launch: application.launch,
            actions: application.actions,
        })
        .collect();
    (apps, None)
}

fn signal_catalog_change(sender: &async_channel::Sender<()>) {
    let _ = sender.try_send(());
}

/// Resolve every app's category concurrently (each read is an independent
/// subprocess), preserving input order.
#[cfg(target_os = "macos")]
fn parallel_categorize(pairs: &[(String, PathBuf)]) -> Vec<Category> {
    let n = pairs.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = 8.min(n);
    let chunk = n.div_ceil(workers);
    let mut result = vec![Category::Other; n];
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (ci, slice) in pairs.chunks(chunk).enumerate() {
            handles.push((
                ci,
                s.spawn(move || {
                    slice
                        .iter()
                        .map(|(name, path)| categorize(name, path))
                        .collect::<Vec<_>>()
                }),
            ));
        }
        for (ci, h) in handles {
            if let Ok(part) = h.join() {
                let start = ci * chunk;
                for (i, c) in part.into_iter().enumerate() {
                    result[start + i] = c;
                }
            }
        }
    });
    result
}

/// The real `LSApplicationCategoryType` from an app's Info.plist, mapped to a
/// bucket — or `None` if the app declares no category.
#[cfg(target_os = "macos")]
fn real_category(path: &Path) -> Option<Category> {
    let info = path.join("Contents/Info");
    let out = Command::new("defaults")
        .arg("read")
        .arg(&info)
        .arg("LSApplicationCategoryType")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    // Apple values look like "public.app-category.developer-tools".
    Some(if s.contains("developer") {
        Category::Developer
    } else if s.contains("game") {
        Category::Games
    } else if s.contains("music")
        || s.contains("video")
        || s.contains("photo")
        || s.contains("entertainment")
        || s.contains("graphics")
    {
        Category::Media
    } else if s.contains("social") || s.contains("news") {
        Category::Internet
    } else if s.contains("utilit") {
        Category::Utilities
    } else if s.contains("productivity")
        || s.contains("business")
        || s.contains("finance")
        || s.contains("reference")
        || s.contains("education")
        || s.contains("weather")
    {
        Category::Productivity
    } else {
        Category::Other
    })
}

/// Derive an App-Library-style category. The app's real declared
/// `LSApplicationCategoryType` wins; only when a bundle declares none do we fall
/// back to install-folder rules and a best-effort name match.
#[cfg(target_os = "macos")]
fn categorize(name: &str, path: &Path) -> Category {
    // Prefer the app's real declared category; fall back to the heuristic only
    // when the bundle declares none.
    if let Some(c) = real_category(path) {
        return c;
    }
    let p = path.to_string_lossy();
    if p.contains("/Utilities/") {
        return Category::Utilities;
    }
    let n = name.to_lowercase();

    const INTERNET: &[&str] = &[
        "safari", "mail", "messages", "facetime", "chrome", "firefox", "edge", "news", "contacts",
        "freeform", "maps",
    ];
    const MEDIA: &[&str] = &[
        "music",
        "tv",
        "photos",
        "podcasts",
        "quicktime",
        "books",
        "voice memos",
        "image capture",
        "photo booth",
        "garageband",
        "imovie",
    ];
    const PRODUCTIVITY: &[&str] = &[
        "calendar",
        "notes",
        "reminders",
        "numbers",
        "pages",
        "keynote",
        "stocks",
        "weather",
        "calculator",
        "dictionary",
        "home",
        "clock",
        "shortcuts",
        "preview",
        "stickies",
        "textedit",
        "font book",
    ];
    const DEVELOPER: &[&str] = &[
        "xcode",
        "terminal",
        "script editor",
        "automator",
        "console",
        "instruments",
        "simulator",
        "visual studio",
        "code",
    ];
    const GAMES: &[&str] = &["chess", "game center"];

    let any = |list: &[&str]| list.iter().any(|k| n.contains(k));

    if any(GAMES) {
        Category::Games
    } else if any(DEVELOPER) {
        Category::Developer
    } else if any(INTERNET) {
        Category::Internet
    } else if any(MEDIA) {
        Category::Media
    } else if any(PRODUCTIVITY) {
        Category::Productivity
    } else if name == "System Settings"
        || name == "App Store"
        || name == "Find My"
        || name == "Passwords"
        || name == "Tips"
        || p.starts_with("/System/Applications")
    {
        Category::System
    } else {
        Category::Other
    }
}

#[cfg(not(target_os = "macos"))]
fn categorize_desktop(categories: &[String]) -> Category {
    let has = |names: &[&str]| {
        categories
            .iter()
            .any(|category| names.contains(&category.as_str()))
    };
    if has(&["Game"]) {
        Category::Games
    } else if has(&["Development", "IDE", "Building", "Debugger"]) {
        Category::Developer
    } else if has(&["Network", "WebBrowser", "Email", "InstantMessaging"]) {
        Category::Internet
    } else if has(&["AudioVideo", "Audio", "Video", "Graphics", "Photography"]) {
        Category::Media
    } else if has(&["Office", "Education", "Science", "Finance"]) {
        Category::Productivity
    } else if has(&["Settings", "System"]) {
        Category::System
    } else if has(&["Utility", "FileTools", "Archiving"]) {
        Category::Utilities
    } else {
        Category::Other
    }
}

#[cfg(target_os = "macos")]
fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join("Library/Caches/rmac-app-drawer");
    std::fs::create_dir_all(&dir).ok();
    dir
}

#[cfg(target_os = "macos")]
fn hydrate_icons(apps: &mut [App]) {
    let cache = cache_dir();
    for app in apps {
        app.icon = extract_icon(&app.name, &app.path, &cache);
    }
}

/// Find an app's `.icns`, convert to a cached 128px PNG (cached across launches).
#[cfg(target_os = "macos")]
fn extract_icon(name: &str, app: &Path, cache: &Path) -> Option<PathBuf> {
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let out = cache.join(format!("{safe}.png"));
    if out.exists() {
        return Some(out);
    }
    let icns = icns_path(app)?;
    let ok = Command::new("sips")
        .args([
            "-s",
            "format",
            "png",
            "-Z",
            "128",
            icns.to_str()?,
            "--out",
            out.to_str()?,
        ])
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok && out.exists() {
        Some(out)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn icns_path(app: &Path) -> Option<PathBuf> {
    let resources = app.join("Contents/Resources");
    let plist = app.join("Contents/Info.plist");

    // Preferred: the icon named by CFBundleIconFile.
    if let Ok(out) = Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Print :CFBundleIconFile",
            plist.to_string_lossy().as_ref(),
        ])
        .output()
    {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            let mut p = resources.join(&name);
            if p.extension().is_none() {
                p.set_extension("icns");
            }
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Fallback: the largest `.icns` in Resources.
    std::fs::read_dir(&resources)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("icns"))
        .max_by_key(|p| p.metadata().map(|m| m.len()).unwrap_or(0))
}

fn key_bindings() -> [KeyBinding; 6] {
    [
        KeyBinding::new(
            rmac_ui::shortcuts::LEFT.keystroke,
            MoveLeft,
            Some("AppDrawer"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::RIGHT.keystroke,
            MoveRight,
            Some("AppDrawer"),
        ),
        KeyBinding::new(rmac_ui::shortcuts::UP.keystroke, MoveUp, Some("AppDrawer")),
        KeyBinding::new(
            rmac_ui::shortcuts::DOWN.keystroke,
            MoveDown,
            Some("AppDrawer"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::ENTER.keystroke,
            Launch,
            Some("AppDrawer"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::ESCAPE.keystroke,
            ClearSearch,
            Some("AppDrawer"),
        ),
    ]
}

fn notify_ready() -> Result<(), String> {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return Ok(());
    }
    let status = std::process::Command::new("/usr/bin/systemd-notify")
        .arg("--ready")
        .arg("--status=App Drawer shortcut endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected App Drawer readiness".to_owned())
}

fn route_shortcut(cx: &mut GpuiApp) {
    let active = cx.read_global::<AppDrawerService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let dismissed = cx
                .update_window(active.window, |_, window, cx| {
                    view.update(cx, |view, cx| view.dismiss(window, cx));
                })
                .is_ok();
            cx.update_global::<AppDrawerService, _>(|service, _| service.active = None);
            if dismissed {
                return;
            }
        }
        cx.update_global::<AppDrawerService, _>(|service, _| service.active = None);
    }

    let token = cx.update_global::<AppDrawerService, _>(|service, _| {
        service.next_token = service.next_token.wrapping_add(1).max(1);
        service.next_token
    });
    let mut drawer = None;
    let handle = cx.open_window(
        rmac_ui::window_options_for_app(rmac_ui::app_id::APP_DRAWER, 1080.0, 720.0),
        |window, cx| {
            rmac_ui::prepare_surface_window(window, cx);
            let view = cx.new(|cx| AppDrawer::new(Some(token), window, cx));
            drawer = Some(view.downgrade());
            cx.new(|cx| Root::new(view, window, cx))
        },
    );
    if let (Ok(handle), Some(view)) = (handle, drawer) {
        cx.update_global::<AppDrawerService, _>(|service, _| {
            service.active = Some(ActiveDrawer {
                token,
                view,
                window: handle.into(),
            });
        });
        cx.activate(true);
    }
}

fn run_service(show_on_start: bool) {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut GpuiApp| {
            rmac_ui::init_application(cx);
            cx.bind_keys(key_bindings());
            cx.set_global(AppDrawerService {
                active: None,
                next_token: 0,
            });

            let (shortcut_tx, shortcut_rx) = async_channel::bounded(8);
            let (ready_tx, ready_rx) = async_channel::bounded(1);
            let shortcut_done = cx.background_executor().spawn(async move {
                rmac_shortcuts::watch_dispatches_ready(
                    rmac_shortcuts::ShortcutId("app-drawer".into()),
                    shortcut_tx,
                    ready_tx,
                )
                .await
            });
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                let consume = async {
                    while shortcut_rx.recv().await.is_ok() {
                        if cx.update(route_shortcut).is_err() {
                            return Err("App Drawer application context stopped".to_owned());
                        }
                    }
                    Ok::<(), String>(())
                };
                let watcher = async { shortcut_done.await.map_err(|error| error.to_string()) };
                let readiness = async {
                    ready_rx
                        .recv()
                        .await
                        .map_err(|_| "App Drawer endpoint stopped before readiness".to_owned())?;
                    blocking::unblock(notify_ready).await
                };
                if let Err(error) = futures_util::try_join!(watcher, consume, readiness) {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            })
            .detach();

            if show_on_start {
                route_shortcut(cx);
            }
        });
}

fn main() {
    match run_mode(std::env::args().skip(1)) {
        RunMode::Service { show_on_start } => run_service(show_on_start),
        RunMode::Standalone => rmac_ui::boot_app(
            rmac_ui::app_id::APP_DRAWER,
            "Applications",
            1080.0,
            720.0,
            |window, cx| {
                cx.bind_keys(key_bindings());
                AppDrawer::new(None, window, cx)
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_change_bursts_coalesce() {
        let (sender, receiver) = async_channel::bounded(1);

        signal_catalog_change(&sender);
        signal_catalog_change(&sender);
        signal_catalog_change(&sender);

        assert_eq!(receiver.len(), 1);
    }
}
