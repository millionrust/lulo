use super::*;

impl FinderView {
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
            self.operation_error = Some("A Finder window can contain up to 16 tabs".into());
            cx.notify();
            return;
        }
        self.save_tab();
        self.trash_view = false;
        self.applications_view = false;
        self.tabs.push(Tab {
            cwd: self.home.clone(),
            identity: None,
            back: Vec::new(),
            fwd: Vec::new(),
        });
        self.active = self.tabs.len() - 1;
        self.load_tab(self.active);
        self.persist_finder_state();
        self.reload(cx);
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

    pub(super) fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() || (path == self.cwd && !self.trash_view && !self.applications_view) {
            return;
        }
        self.trash_view = false;
        self.applications_view = false;
        self.back.push(self.cwd.clone());
        self.fwd.clear();
        self.cwd = path;
        self.cwd_identity = None;
        self.persist_finder_state();
        self.reload(cx);
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
        let paths: Vec<(bool, PathBuf)> = self
            .selected
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .map(|entry| (entry.is_dir, entry.path.clone()))
            .collect();
        if let [(true, directory)] = paths.as_slice() {
            self.navigate(directory.clone(), cx);
        } else {
            self.open_paths(paths.into_iter().map(|(_, path)| path).collect(), cx);
        }
    }

    pub(super) fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
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
