//! App Drawer catalog, selection, and launch controller.

mod lifecycle;
mod render;

#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, FocusHandle, Pixels, Point, SharedString, Window};
use rmac_app_drawer::accessibility::{
    project_app_drawer, AppDrawerAccessibilitySnapshot, ApplicationCategory, DrawerFeedback,
    DrawerProjectionState, DrawerView, OPEN_ACTION_NAME, SHOW_IN_FOLDER_ACTION_NAME,
};
use rmac_ui::InputState;

use crate::catalog::{self, App, Category};
use crate::service;
use crate::{OpenApp, RevealInFinder};

pub(crate) const DRAWER_WIDTH: f32 = 846.0;
pub(crate) const DRAWER_HEIGHT: f32 = 800.0;
const TILE_W: f32 = 104.0;
const ICON: f32 = 60.0;
const ROW_ICON: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Grid,
    List,
}

impl From<ViewMode> for DrawerView {
    fn from(view: ViewMode) -> Self {
        match view {
            ViewMode::Grid => Self::Grid,
            ViewMode::List => Self::List,
        }
    }
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
    /// macOS does not paint a selection merely because the surface opened.
    /// The highlight appears only after directional keyboard navigation.
    selection_visible: bool,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<rmac_ui::ContextMenuState>,
    /// Columns in the grid as last laid out — used for up/down navigation.
    cols: usize,
    catalog_error: Option<SharedString>,
    action_error: Option<SharedString>,
    launching: bool,
    _catalog_watcher: Option<rmac_apps::CatalogWatcher>,
}

impl AppDrawer {
    /// Indices into `self.apps` that pass the current category + search filter.
    fn visible_indices(&self, cx: &gpui::App) -> Vec<usize> {
        self.search_matching_indices(cx)
            .into_iter()
            .filter(|index| self.filter.is_none_or(|c| self.apps[*index].category == c))
            .collect()
    }

    /// Catalog-order matches for the private query. The query never crosses
    /// the accessibility boundary; only these controller-owned indices do.
    fn search_matching_indices(&self, cx: &gpui::App) -> Vec<usize> {
        let query = self.query.read(cx).value().trim().to_lowercase();
        self.apps
            .iter()
            .enumerate()
            .filter(|(_, app)| query.is_empty() || app.search_text.contains(&query))
            .map(|(index, _)| index)
            .collect()
    }

    /// The set of categories actually present after the *search* filter — used
    /// to build the category bar so we never show an empty bucket.
    fn present_categories(&self, cx: &gpui::App) -> Vec<Category> {
        let search_matches = self.search_matching_indices(cx);
        Category::ORDER
            .into_iter()
            .filter(|c| {
                search_matches
                    .iter()
                    .any(|index| self.apps[*index].category == *c)
            })
            .collect()
    }

    /// Exact adapter-ready snapshot. Pinned GPUI cannot publish this tree yet,
    /// so the method remains dormant until the A5/A6 framework gate is chosen.
    #[allow(dead_code)]
    fn accessibility_snapshot(
        &self,
        cx: &gpui::App,
    ) -> Result<
        AppDrawerAccessibilitySnapshot,
        rmac_app_drawer::accessibility::AccessibilityProjectionError,
    > {
        let search_matches = self.search_matching_indices(cx);
        let visible = self.visible_indices(cx);
        let selected_index =
            (!visible.is_empty()).then_some(self.selected.min(visible.len().saturating_sub(1)));
        let feedback = if self.launching {
            DrawerFeedback::Busy
        } else if let Some(error) = self.action_error.as_ref().or(self.catalog_error.as_ref()) {
            DrawerFeedback::Error(error.as_ref())
        } else {
            DrawerFeedback::Ready
        };
        project_app_drawer(
            &self.apps,
            &search_matches,
            &visible,
            DrawerProjectionState {
                view: self.view.into(),
                filter: self.filter.map(ApplicationCategory::from),
                selected_index,
                search_active: !self.query.read(cx).value().trim().is_empty(),
                context_menu_open: self.menu_at.is_some(),
                feedback,
            },
        )
    }

    fn move_by(&mut self, dx: isize, dy: isize, cx: &mut Context<Self>) {
        let count = self.visible_indices(cx).len();
        if count == 0 {
            return;
        }
        if !self.selection_visible {
            self.selected = 0;
            self.selection_visible = true;
            cx.notify();
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
        self.selection_visible = true;
        cx.notify();
    }

    fn launch_selected(&mut self, cx: &mut Context<Self>) {
        if !self.selection_visible && self.query.read(cx).value().trim().is_empty() {
            return;
        }
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
            OPEN_ACTION_NAME,
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
            .item(SHOW_IN_FOLDER_ACTION_NAME, Box::new(RevealInFinder))
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.read(cx).value().trim().is_empty() {
            self.query.update(cx, |st, cx| st.set_value("", window, cx));
            self.selected = 0;
            self.selection_visible = false;
            self.focus.focus(window);
        } else if self.filter.take().is_some() {
            self.selected = 0;
            self.selection_visible = false;
        } else {
            self.dismiss(window, cx);
            return;
        }
        cx.notify();
    }

    fn set_filter(&mut self, filter: Option<Category>, cx: &mut Context<Self>) {
        self.filter = filter;
        self.selected = 0;
        self.selection_visible = false;
        cx.notify();
    }
}
