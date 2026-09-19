//! Apps window, search observation, and live-catalog lifecycle.

use super::*;

impl AppDrawer {
    pub(crate) fn new(
        service_token: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (apps, mut catalog_error) = catalog::scan();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search Apps"));

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
            this.selection_visible = false;
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
        focus.focus(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                this.dismiss(window, cx);
            }
        })
        .detach();

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
                        format!("Apps loaded, but live updates are unavailable: {error}").into(),
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
                        let (apps, error) = catalog::scan();
                        #[cfg(target_os = "macos")]
                        let apps = {
                            let mut apps = apps;
                            catalog::hydrate_icons(&mut apps);
                            apps
                        };
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
            selection_visible: false,
            menu_at: None,
            cols: 6,
            catalog_error,
            action_error: None,
            launching: false,
            was_active: false,
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
}
