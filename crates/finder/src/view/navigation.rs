use super::*;

impl FinderView {
    pub(super) fn toggle_tab_bar(&mut self, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            return;
        }
        self.show_tab_bar = !self.show_tab_bar;
        cx.notify();
    }

    /// View ▸ Show All Tabs, ⇧⌘\.
    pub(super) fn toggle_show_all_tabs(&mut self, cx: &mut Context<Self>) {
        self.show_all_tabs = !self.show_all_tabs;
        cx.notify();
    }

    pub(super) fn selected_folder(&self) -> Option<PathBuf> {
        if self.trash_view || self.applications_view {
            return None;
        }
        let path = self.selected_entry()?.path.clone();
        path.is_dir().then_some(path)
    }

    pub(super) fn open_selection_in_new_tab(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_folder() else {
            return;
        };
        if self.tabs.len() >= MAX_RESTORED_TABS {
            self.operation_error = Some("A Files window can contain up to 16 tabs".into());
            cx.notify();
            return;
        }
        self.new_tab(cx);
        self.navigate(path, cx);
    }

    fn open_selected_folder_window(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.selected_folder() else {
            return false;
        };
        let Some(path) = path.to_str().map(str::to_owned) else {
            self.operation_error = Some("The folder path cannot open a new window".into());
            cx.notify();
            return false;
        };
        if rmac_ui::open_another_window(vec!["--path".to_owned(), path], cx) {
            true
        } else {
            self.operation_error = Some("Files could not open another window".into());
            cx.notify();
            false
        }
    }

    pub(super) fn open_selection_in_new_window(&mut self, cx: &mut Context<Self>) {
        self.open_selected_folder_window(cx);
    }

    pub(super) fn open_selection_in_new_window_and_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_selected_folder_window(cx) {
            self.close_finder_window(window, cx);
        }
    }

    pub(super) fn open_parent_in_new_window(&mut self, cx: &mut Context<Self>) {
        let Some(parent) = self.cwd.parent().and_then(Path::to_str) else {
            return;
        };
        if !rmac_ui::open_another_window(vec!["--path".to_owned(), parent.to_owned()], cx) {
            self.operation_error = Some("Files could not open another window".into());
            cx.notify();
        }
    }

    /// Persist the active tab's live navigation state into the tab list.
    fn save_tab(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.cwd = self.cwd.clone();
            tab.identity = self.cwd_identity;
            tab.back = self.back.clone();
            tab.fwd = self.fwd.clone();
        }
    }

    /// Load tab `index`'s state into the live fields.
    fn load_tab(&mut self, index: usize) {
        if let Some(tab) = self.tabs.get(index) {
            self.cwd = tab.cwd.clone();
            self.cwd_identity = tab.identity;
            self.back = tab.back.clone();
            self.fwd = tab.fwd.clone();
        }
    }

    pub(super) fn new_tab(&mut self, cx: &mut Context<Self>) {
        if self.tabs.len() >= MAX_RESTORED_TABS {
            self.operation_error = Some("A Files window can contain up to 16 tabs".into());
            cx.notify();
            return;
        }
        self.save_tab();
        self.trash_view = false;
        self.applications_view = false;
        self.tabs.push(Tab {
            cwd: self.cwd.clone(),
            identity: None,
            back: Vec::new(),
            fwd: Vec::new(),
        });
        self.active = self.tabs.len() - 1;
        self.load_tab(self.active);
        self.persist_finder_state();
        self.reload(cx);
    }

    /// ⌘W: closes the current tab, or the window itself when it has only
    /// one, as the Mac's Close Tab does. Before this, ⌘W with one tab did
    /// nothing at all — only the traffic light could close the window.
    pub(super) fn close_tab_or_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 {
            self.close_finder_window(window, cx);
            return;
        }
        let active = self.active;
        self.close_tab(active, cx);
    }

    pub(super) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        let was_active = index == self.active;
        if was_active {
            self.save_tab();
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > index {
            self.active -= 1;
        }
        if was_active {
            self.trash_view = false;
            self.applications_view = false;
            self.load_tab(self.active);
            self.persist_finder_state();
            self.reload(cx);
        } else {
            self.persist_finder_state();
            cx.notify();
        }
    }

    pub(super) fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.save_tab();
        self.trash_view = false;
        self.applications_view = false;
        self.active = index;
        self.load_tab(index);
        self.persist_finder_state();
        self.reload(cx);
    }

    pub(super) fn select_adjacent_tab(&mut self, offset: isize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 {
            return;
        }
        let count = self.tabs.len() as isize;
        let index = (self.active as isize + offset).rem_euclid(count) as usize;
        self.select_tab(index, cx);
    }

    pub(super) fn go_home(&mut self, cx: &mut Context<Self>) {
        self.navigate(self.home.clone(), cx);
    }

    pub(super) fn go_downloads(&mut self, cx: &mut Context<Self>) {
        let downloads = self.home.join("Downloads");
        if downloads.is_dir() {
            self.navigate(downloads, cx);
        }
    }

    pub(super) fn go_shared(&mut self, cx: &mut Context<Self>) {
        if let Some(shared) = rmac_finder::places::shared_folder(&self.home) {
            self.navigate(shared.path, cx);
        }
    }

    /// Go ▸ Desktop, ⇧⌘D.
    pub(super) fn go_desktop(&mut self, cx: &mut Context<Self>) {
        let desktop = self.home.join("Desktop");
        if desktop.is_dir() {
            self.navigate(desktop, cx);
        }
    }

    /// Go ▸ Documents, ⇧⌘O.
    pub(super) fn go_documents(&mut self, cx: &mut Context<Self>) {
        let documents = self.home.join("Documents");
        if documents.is_dir() {
            self.navigate(documents, cx);
        }
    }

    /// Go ▸ Library. macOS's `~/Library` holds per-app support files,
    /// caches and preferences; Linux splits that across the XDG base
    /// directories, so this opens `$XDG_DATA_HOME` (default `~/.local/
    /// share`) — the closest single analogue, and where Lulo's own apps
    /// keep their "Application Support"-equivalent files.
    pub(super) fn go_library(&mut self, cx: &mut Context<Self>) {
        let library = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| self.home.join(".local/share"));
        if library.is_dir() {
            self.navigate(library, cx);
        }
    }

    pub(super) fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() || (path == self.cwd && !self.trash_view && !self.applications_view) {
            return;
        }
        self.trash_view = false;
        self.applications_view = false;
        self.browse_view = self.current_options().browse_in_view.then_some(self.view);
        self.back.push(self.cwd.clone());
        self.fwd.clear();
        self.record_recent_folder(path.clone());
        self.cwd = path;
        self.cwd_identity = None;
        self.persist_finder_state();
        self.reload(cx);
    }

    /// Go ▸ Recent Folders: remember a freshly-navigated-to folder,
    /// most-recent first, capped and de-duplicated. Only `navigate()` calls
    /// this — plain Back/Forward browsing (`go_back`/`go_forward`) revisits
    /// folders already on the list rather than adding new "recent" ones, the
    /// same distinction the Mac draws.
    fn record_recent_folder(&mut self, path: PathBuf) {
        self.recent_folders.retain(|existing| existing != &path);
        self.recent_folders.insert(0, path);
        self.recent_folders.truncate(RECENT_FOLDERS_CAP);
    }

    /// Go ▸ Recent Folders: opens the dynamic list as a standalone popup
    /// (see `MenuPurpose::RecentFolders`).
    pub(super) fn show_recent_folders_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_purpose = MenuPurpose::RecentFolders;
        self.menu_at = Some(rmac_ui::ContextMenuState::open(
            gpui::point(px(16.0), px(44.0)),
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    /// Go ▸ Recent Folders ▸ Clear Menu.
    pub(super) fn clear_recent_folders(&mut self, cx: &mut Context<Self>) {
        self.recent_folders.clear();
        self.menu_at = None;
        cx.notify();
    }

    /// One row of the Go ▸ Recent Folders popup.
    pub(super) fn open_recent_folder(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(path) = self.recent_folders.get(index).cloned() {
            self.navigate(path, cx);
        }
    }

    pub(super) fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.trash_view || self.applications_view {
            self.trash_view = false;
            self.applications_view = false;
            self.reload(cx);
            return;
        }
        if let Some(path) = self.back.pop() {
            self.fwd.push(self.cwd.clone());
            self.cwd = path;
            self.cwd_identity = None;
            self.persist_finder_state();
            self.reload(cx);
        }
    }

    pub(super) fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.trash_view || self.applications_view {
            self.trash_view = false;
            self.applications_view = false;
            if self.fwd.is_empty() {
                self.reload(cx);
                return;
            }
        }
        if let Some(path) = self.fwd.pop() {
            self.back.push(self.cwd.clone());
            self.cwd = path;
            self.cwd_identity = None;
            self.persist_finder_state();
            self.reload(cx);
        }
    }

    pub(super) fn go_up(&mut self, cx: &mut Context<Self>) {
        if self.trash_view || self.applications_view {
            self.trash_view = false;
            self.applications_view = false;
            self.reload(cx);
            return;
        }
        if let Some(parent) = self.cwd.parent().map(Path::to_path_buf) {
            self.pending_select = Some(self.cwd.clone());
            self.navigate(parent, cx);
        }
    }

    pub(super) fn open_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Restore the item before opening it".into());
            cx.notify();
            return;
        }
        let Some(entry) = self.entries.get(index).cloned() else {
            return;
        };
        if let Some(application) = entry.application {
            self.launch_applications(vec![application.launch], cx);
            return;
        }
        if entry.is_dir {
            self.navigate(entry.path, cx);
        } else {
            self.open_paths(vec![entry.path], cx);
        }
    }

    pub(super) fn open_selected(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Restore items before opening them".into());
            cx.notify();
            return;
        }
        let applications = self
            .selected
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .filter_map(|entry| entry.application.as_ref())
            .map(|application| application.launch.clone())
            .collect::<Vec<_>>();
        if !applications.is_empty() {
            self.launch_applications(applications, cx);
            return;
        }
        let paths: Vec<(bool, PathBuf)> =
            if self.view == ViewMode::Column && !self.applications_view {
                self.column_selection
                    .as_ref()
                    .map(|entry| vec![(entry.is_dir, entry.path.clone())])
                    .unwrap_or_default()
            } else {
                self.selected
                    .iter()
                    .filter_map(|&index| self.entries.get(index))
                    .map(|entry| (entry.is_dir, entry.path.clone()))
                    .collect()
            };
        if let [(true, directory)] = paths.as_slice() {
            self.navigate(directory.clone(), cx);
        } else {
            self.open_paths(paths.into_iter().map(|(_, path)| path).collect(), cx);
        }
    }

    pub(super) fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        // Archive Utility's job on the Mac: expand next to the archive.
        let (archives, paths): (Vec<PathBuf>, Vec<PathBuf>) = paths
            .into_iter()
            .partition(|path| rmac_archive::format_of(path).is_some() && path.is_file());
        if !archives.is_empty() {
            self.expand_archives(archives, cx);
        }
        if paths.is_empty() {
            return;
        }
        self.operation_error = None;
        self.open_generation = self.open_generation.wrapping_add(1);
        let generation = self.open_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let total = paths.len();
            let mut failures = Vec::new();
            for path in paths {
                if let Err(error) = rmac_app_launch::open_item(path).await {
                    failures.push(error.to_string());
                }
            }
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.open_generation != generation {
                    return;
                }
                if let Some(first) = failures.first() {
                    this.operation_error = Some(
                        if failures.len() == 1 {
                            first.clone()
                        } else {
                            format!(
                                "{first} (and {} more of {total} items could not be opened)",
                                failures.len() - 1
                            )
                        }
                        .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn launch_applications(
        &mut self,
        applications: Vec<rmac_apps::LaunchSpec>,
        cx: &mut Context<Self>,
    ) {
        if applications.is_empty() {
            return;
        }
        self.operation_error = None;
        self.open_generation = self.open_generation.wrapping_add(1);
        let generation = self.open_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let total = applications.len();
            let mut failures = Vec::new();
            for application in applications {
                if let Err(error) = rmac_app_launch::launch(application).await {
                    failures.push(error.to_string());
                }
            }
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.open_generation != generation {
                    return;
                }
                if let Some(first) = failures.first() {
                    this.operation_error = Some(
                        if failures.len() == 1 {
                            format!("Could not open application: {first}")
                        } else {
                            format!(
                                "Could not open application: {first} (and {} more of {total})",
                                failures.len() - 1
                            )
                        }
                        .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }
}
