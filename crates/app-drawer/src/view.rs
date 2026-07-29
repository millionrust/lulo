//! App Drawer catalog, selection, and launch controller.

mod render;

#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, FocusHandle, Pixels, Point, SharedString, Window};
use rmac_ui::InputState;

use crate::catalog::{self, App, Category};
use crate::service;
use crate::{OpenApp, RevealInFinder};

const TILE_W: f32 = 116.0;
const ICON: f32 = 60.0;
const ROW_ICON: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Grid,
    List,
}

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = app_drawer, no_json)]
struct LaunchDesktopAction {
    launch: rmac_apps::LaunchSpec,
}

pub(crate) struct AppDrawer {
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

impl AppDrawer {
    pub(crate) fn new(
        service_token: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (apps, mut catalog_error) = catalog::scan();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        if let Some(token) = service_token {
            cx.on_release(move |_, cx| {
                service::release(token, cx);
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
                        let cache = catalog::cache_dir();
                        snapshot
                            .into_iter()
                            .map(|(name, path)| {
                                let icon = catalog::extract_icon(&name, &path, &cache);
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
            catalog::signal_change(&catalog_events);
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
                        let (mut apps, error) = catalog::scan();
                        #[cfg(target_os = "macos")]
                        catalog::hydrate_icons(&mut apps);
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

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(token) = self.service_token {
            service::release(token, cx);
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
}
