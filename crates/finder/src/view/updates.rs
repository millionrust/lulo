use super::*;

impl FinderView {
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.reload_trash(cx);
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            self.delete_confirmation = None;
        }
        self.reload_inner(cx, true);
    }

    /// Refresh after a watcher event without spawning `df`; free space changes
    /// slowly and is refreshed on navigation and explicit file operations.
    pub(super) fn reload_after_event(&mut self, hints: FilesystemHints, cx: &mut Context<Self>) {
        if self.trash_view {
            return;
        }
        if hints.watch_error {
            if let Some(watcher) = self.watcher.as_mut() {
                if let Some(watched) = self.watched.take() {
                    let _ = watcher.unwatch(&watched);
                }
                if let Some(parent) = self.watched_parent.take() {
                    let _ = watcher.unwatch(&parent);
                }
            }
            self.watcher = None;
            if self.operation_error.is_none() {
                self.operation_error = Some(FILESYSTEM_WATCH_INTERRUPTED_MESSAGE.into());
            }
        }
        let Some(expected) = self.cwd_identity else {
            self.reload_inner(cx, false);
            return;
        };
        if hints.renames.is_empty() {
            self.reload_inner(cx, false);
            return;
        }

        self.directory_generation = self.directory_generation.wrapping_add(1);
        let generation = self.directory_generation;
        let current = self.cwd.clone();
        let renames = hints.renames;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let resolution = cx
                .background_executor()
                .spawn({
                    let current = current.clone();
                    async move { directory_state::renamed_path(&current, expected, &renames) }
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.directory_generation != generation
                    || this.cwd != current
                    || this.cwd_identity != Some(expected)
                {
                    return;
                }
                if let Some(resolution) = resolution {
                    this.rewrite_navigation_prefix(&resolution.old, &resolution.new);
                    this.cwd = resolution.current;
                    this.operation_notice =
                        Some("The current folder was renamed; Files followed it".into());
                }
                this.reload_inner(cx, false);
            });
        })
        .detach();
    }

    fn rewrite_navigation_prefix(&mut self, old: &Path, new: &Path) {
        self.cwd = directory_state::rewrite_prefix(&self.cwd, old, new);
        for path in &mut self.back {
            *path = directory_state::rewrite_prefix(path, old, new);
        }
        for path in &mut self.fwd {
            *path = directory_state::rewrite_prefix(path, old, new);
        }
        for tab in &mut self.tabs {
            tab.cwd = directory_state::rewrite_prefix(&tab.cwd, old, new);
            for path in &mut tab.back {
                *path = directory_state::rewrite_prefix(path, old, new);
            }
            for path in &mut tab.fwd {
                *path = directory_state::rewrite_prefix(path, old, new);
            }
        }
    }

    pub(super) fn reload_trash(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", test))]
        {
            self.cancel_search();
            self.result_title = Some("Trash".into());
            self.search_summary = None;
            self.search_relevance_order = false;
            self.selected.clear();
            self.anchor = None;
            self.renaming = None;
            self.trash_generation = self.trash_generation.wrapping_add(1);
            let generation = self.trash_generation;
            let key = self.sort_key;
            let asc = self.sort_asc;
            let Some(store) = self.trash_store.clone() else {
                self.entries.clear();
                self.trash_items.clear();
                self.operation_error = Some(
                    if self.trash_loading {
                        "Files is still verifying Trash recovery"
                    } else {
                        "Trash is unavailable"
                    }
                    .into(),
                );
                cx.notify();
                return;
            };
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let items = store.list()?;
                        let mut entries = Vec::with_capacity(items.len());
                        for item in &items {
                            let mut entry = entry_for(item.data_path()).ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::WouldBlock,
                                    "Trash changed while it was listed",
                                )
                            })?;
                            entry.name = item
                                .original_path
                                .file_name()
                                .unwrap_or(item.name.as_os_str())
                                .to_string_lossy()
                                .into_owned()
                                .into();
                            entry.modified = item.deleted_at.clone().into();
                            entries.push(entry);
                        }
                        sort_entries(&mut entries, key, asc);
                        Ok::<_, std::io::Error>((items, entries))
                    })
                    .await;
                let _ = this.update(cx, |this: &mut FinderView, cx| {
                    if !this.trash_view || this.trash_generation != generation {
                        return;
                    }
                    match result {
                        Ok((items, entries)) => {
                            this.trash_items = items;
                            this.entries = entries;
                            this.free_bytes = None;
                        }
                        Err(error) => {
                            this.entries.clear();
                            this.trash_items.clear();
                            this.operation_error = Some(
                                match error.kind() {
                                    std::io::ErrorKind::WouldBlock => {
                                        "Trash is busy or changed; try again"
                                    }
                                    _ => "Trash could not be verified safely",
                                }
                                .into(),
                            );
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.entries.clear();
            self.operation_error = Some("Trash browsing is available on Linux".into());
            cx.notify();
        }
    }

    fn reload_inner(&mut self, cx: &mut Context<Self>, refresh_free_space: bool) {
        self.cancel_search();
        self.result_title = None;
        self.search_summary = None;
        self.search_relevance_order = false;
        self.col_stack = vec![self.cwd.clone()];
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.cwd = self.cwd.clone();
            t.identity = self.cwd_identity;
        }
        // Reconfigure the watcher only after navigation. Re-watching the same
        // directory in response to its own event can create a reload storm.
        let mut watch_failed = false;
        let rebuilding_watcher = self.watcher.is_none();
        if rebuilding_watcher {
            self.watcher = filesystem_watcher(
                self.filesystem_events.clone(),
                self.filesystem_hints.clone(),
            )
            .ok();
            watch_failed = self.watcher.is_none();
        }
        let expected_parent = self
            .cwd
            .parent()
            .filter(|parent| *parent != self.cwd)
            .map(Path::to_path_buf);
        if self.watched.as_ref() != Some(&self.cwd)
            || self.watched_parent.as_ref() != expected_parent.as_ref()
        {
            if let Some(w) = self.watcher.as_mut() {
                if let Some(old) = self.watched.take() {
                    let _ = w.unwatch(&old);
                }
                if let Some(old_parent) = self.watched_parent.take() {
                    let _ = w.unwatch(&old_parent);
                }
                if w.watch(&self.cwd, RecursiveMode::NonRecursive).is_ok() {
                    self.watched = Some(self.cwd.clone());
                } else {
                    watch_failed = true;
                }
                if let Some(parent) = expected_parent {
                    if w.watch(&parent, RecursiveMode::NonRecursive).is_ok() {
                        self.watched_parent = Some(parent);
                    } else {
                        watch_failed = true;
                    }
                }
            }
        }
        if watch_failed
            && self.operation_error.as_ref().is_none_or(|message| {
                message.as_ref() == FILESYSTEM_WATCH_INTERRUPTED_MESSAGE
                    || message.as_ref() == FILESYSTEM_WATCH_UNAVAILABLE_MESSAGE
            })
        {
            self.operation_error = Some(FILESYSTEM_WATCH_UNAVAILABLE_MESSAGE.into());
        } else if rebuilding_watcher
            && self.operation_error.as_ref().is_some_and(|message| {
                message.as_ref() == FILESYSTEM_WATCH_INTERRUPTED_MESSAGE
                    || message.as_ref() == FILESYSTEM_WATCH_UNAVAILABLE_MESSAGE
            })
        {
            self.operation_error = None;
            self.operation_notice = Some("Live folder updates resumed".into());
        }

        let path = self.cwd.clone();
        let home = self.home.clone();
        let expected_identity = self.cwd_identity;
        self.directory_generation = self.directory_generation.wrapping_add(1);
        let generation = self.directory_generation;
        let stalled_path = path.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(DIRECTORY_STALL_NOTICE_DELAY)
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.cwd == stalled_path
                    && this.directory_generation == generation
                    && this.operation_error.is_none()
                {
                    this.operation_error = Some(DIRECTORY_STALL_NOTICE.into());
                    cx.notify();
                }
            });
        })
        .detach();
        // Compared on completion so a slow read for a directory we've since
        // navigated away from doesn't clobber the current listing.
        let read_path = path.clone();
        let show_hidden = self.show_hidden;
        let key = self.sort_key;
        let asc = self.sort_asc;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match read_entries_checked(&path, show_hidden, expected_identity) {
                        Ok((identity, mut entries)) => {
                            sort_entries(&mut entries, key, asc);
                            let free = refresh_free_space.then(|| free_space(&path));
                            Ok((identity, entries, free))
                        }
                        Err(error) => Err((
                            error.kind(),
                            directory_state::recovery_parent(&path, &home),
                        )),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Drop stale results from a superseded navigation.
                if this.cwd != read_path || this.directory_generation != generation {
                    return;
                }
                match result {
                    Ok((identity, entries, free)) => {
                        if this
                            .operation_error
                            .as_ref()
                            .is_some_and(|message| message.as_ref() == DIRECTORY_STALL_NOTICE)
                        {
                            this.operation_error = None;
                        }
                        this.cwd_identity = Some(identity);
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            tab.identity = Some(identity);
                        }
                        this.entries = entries;
                        let entry_paths = this
                            .entries
                            .iter()
                            .map(|entry| entry.path.clone())
                            .collect::<BTreeSet<_>>();
                        this.thumbs.retain(|source, thumbnail| {
                            entry_paths.contains(source)
                                && rmac_thumbnails::is_current(source, thumbnail)
                        });
                        if let Some(free) = free {
                            this.free_bytes = free;
                        }
                        this.selected.clear();
                        this.anchor = None;
                        this.renaming = None;
                        cx.notify();
                        this.gen_thumbs(cx);
                    }
                    Err((kind, fallback)) if fallback != this.cwd => {
                        this.entries.clear();
                        this.selected.clear();
                        this.anchor = None;
                        this.renaming = None;
                        this.cwd = fallback;
                        this.cwd_identity = None;
                        this.fwd.clear();
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            tab.cwd = this.cwd.clone();
                            tab.identity = None;
                            tab.fwd.clear();
                        }
                        this.operation_error = Some(
                            match kind {
                                std::io::ErrorKind::PermissionDenied => {
                                    "The folder is no longer accessible; showing its nearest available parent"
                                }
                                _ => {
                                    "The folder moved, disappeared, or was replaced; showing its nearest available parent"
                                }
                            }
                            .into(),
                        );
                        this.reload_inner(cx, true);
                    }
                    Err(_) => {
                        this.entries.clear();
                        this.cwd_identity = None;
                        this.operation_error =
                            Some("This location is unavailable and could not be recovered".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Generate platform thumbnails into the persistent cache off the main thread.
    fn gen_thumbs(&mut self, cx: &mut Context<Self>) {
        let directory = self.cwd.clone();
        let targets: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| {
                !entry.is_dir
                    && rmac_thumbnails::is_supported(&entry.path)
                    && !self.thumbs.get(&entry.path).is_some_and(|thumbnail| {
                        rmac_thumbnails::is_current(&entry.path, thumbnail)
                    })
            })
            .map(|e| e.path.clone())
            .collect();
        if targets.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut generated = Vec::new();
                    let mut first_error = None;
                    let mut failure_count = 0;
                    for path in targets {
                        match rmac_thumbnails::generate(&path) {
                            Ok(thumbnail) => generated.push((path, thumbnail)),
                            Err(error) => {
                                failure_count += 1;
                                first_error.get_or_insert(error);
                            }
                        }
                    }
                    (generated, first_error, failure_count)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                for (p, t) in results.0 {
                    if this.entries.iter().any(|entry| entry.path == p)
                        && rmac_thumbnails::is_current(&p, &t)
                    {
                        this.thumbs.insert(p, t);
                    }
                }
                if this.cwd == directory && this.operation_error.is_none() {
                    this.operation_error = results.1.map(|error| {
                        if results.2 == 1 {
                            format!("Could not generate thumbnail: {error}").into()
                        } else {
                            format!(
                                "Could not generate thumbnail: {error} (and {} more failures)",
                                results.2 - 1
                            )
                            .into()
                        }
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }
}
