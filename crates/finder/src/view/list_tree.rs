use super::bounded_cache::{bound_cache, touch_cache_key, CHILD_ENTRIES_CACHE_CAP};
use super::*;

impl FinderView {
    /// Option-click a disclosure to open or close its whole descendant tree.
    /// The scan is bounded for a low-end machine and never runs on the UI thread.
    pub(super) fn toggle_list_folder_tree(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.expanded.contains(&path) {
            let descendants = self
                .expanded
                .iter()
                .filter(|expanded| expanded.starts_with(&path))
                .cloned()
                .collect::<Vec<_>>();
            for descendant in descendants {
                self.expanded.remove(&descendant);
                if self.watched_children.remove(&descendant) {
                    if let Some(watcher) = self.watcher.as_mut() {
                        let _ = watcher.unwatch(&descendant);
                    }
                }
            }
            self.rebuild_list_entries();
            cx.notify();
            return;
        }
        self.expanded.insert(path.clone());
        self.rebuild_list_entries();
        cx.notify();
        let generation = self.directory_generation;
        let show_hidden = self.show_hidden;
        let key = self.sort_key;
        let asc = self.sort_asc;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let root = path.clone();
            let children = blocking::unblock(move || {
                let mut pending = vec![root];
                let mut children = HashMap::new();
                while let Some(folder) = pending.pop() {
                    if children.len() >= 256 {
                        break;
                    }
                    let Ok((_, mut entries)) = read_entries_checked(&folder, show_hidden, None)
                    else {
                        continue;
                    };
                    sort_entries(&mut entries, key, asc);
                    for entry in &entries {
                        if entry.is_dir
                            && std::fs::symlink_metadata(&entry.path)
                                .is_ok_and(|metadata| !metadata.file_type().is_symlink())
                        {
                            pending.push(entry.path.clone());
                        }
                    }
                    children.insert(folder, entries);
                }
                children
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.directory_generation != generation || !this.expanded.contains(&path) {
                    return;
                }
                for (folder, entries) in children {
                    if let Some(watcher) = this.watcher.as_mut() {
                        if watcher.watch(&folder, RecursiveMode::NonRecursive).is_ok() {
                            this.watched_children.insert(folder.clone());
                        }
                    }
                    this.expanded.insert(folder.clone());
                    touch_cache_key(&mut this.child_entries_order, &folder);
                    this.child_entries.insert(folder, entries);
                }
                let expanded = this.expanded.clone();
                bound_cache(
                    &mut this.child_entries_order,
                    &mut this.child_entries,
                    CHILD_ENTRIES_CACHE_CAP,
                    |key| expanded.contains(key),
                );
                this.rebuild_list_entries();
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn scroll_list_row_into_view(&self, position: usize) {
        if self.view != ViewMode::List {
            return;
        }
        let viewport = f32::from(self.list_scroll.bounds().size.height);
        if viewport <= 0.0 {
            return;
        }
        let row_height = if self
            .entries
            .iter()
            .any(|entry| entry.search_detail.is_some())
        {
            38.0
        } else {
            LIST_ROW_HEIGHT
        };
        let top = (-f32::from(self.list_scroll.offset().y)).max(0.0);
        let row_top = LIST_ROWS_TOP + position as f32 * row_height;
        let target = if row_top < top {
            row_top
        } else if row_top + row_height > top + viewport {
            row_top + row_height - viewport
        } else {
            return;
        };
        self.list_scroll
            .set_offset(gpui::point(px(0.0), px(-target.max(0.0))));
    }

    pub(super) fn list_row_index(&self, path: &Path) -> Option<usize> {
        self.entries.iter().position(|entry| entry.path == path)
    }

    /// Rebuild visible list rows from full paths. Indices are only a view of
    /// this tree; preserve selection by path whenever rows are inserted.
    pub(super) fn rebuild_list_entries(&mut self) {
        let selected = self.selected_paths().into_iter().collect::<BTreeSet<_>>();
        let anchor = self
            .anchor
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.path.clone());
        self.entries.clear();
        self.list_depths.clear();
        if self.view == ViewMode::List && !self.trash_view && !self.applications_view {
            append_rows(
                &self.root_entries,
                0,
                &self.expanded,
                &self.child_entries,
                &mut self.entries,
                &mut self.list_depths,
            );
        } else {
            self.entries.clone_from(&self.root_entries);
        }
        self.selected = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| selected.contains(&entry.path).then_some(index))
            .collect();
        self.anchor = anchor
            .and_then(|path| self.entries.iter().position(|entry| entry.path == path))
            .filter(|index| self.selected.contains(index))
            .or_else(|| self.selected.iter().next().copied());
    }

    pub(super) fn toggle_list_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.expanded.insert(path.clone()) {
            self.expanded.remove(&path);
            if self.watched_children.remove(&path) {
                if let Some(watcher) = self.watcher.as_mut() {
                    let _ = watcher.unwatch(&path);
                }
            }
            self.rebuild_list_entries();
            cx.notify();
            return;
        }
        self.rebuild_list_entries();
        if let Some(watcher) = self.watcher.as_mut() {
            if watcher.watch(&path, RecursiveMode::NonRecursive).is_ok() {
                self.watched_children.insert(path.clone());
            }
        }
        cx.notify();
        let generation = self.directory_generation;
        let show_hidden = self.show_hidden;
        let key = self.sort_key;
        let asc = self.sort_asc;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let read_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    read_entries_checked(&read_path, show_hidden, None).map(|(_, mut entries)| {
                        sort_entries(&mut entries, key, asc);
                        entries
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.directory_generation != generation || !this.expanded.contains(&path) {
                    return;
                }
                match result {
                    Ok(entries) => {
                        touch_cache_key(&mut this.child_entries_order, &path);
                        this.child_entries.insert(path, entries);
                        let expanded = this.expanded.clone();
                        bound_cache(
                            &mut this.child_entries_order,
                            &mut this.child_entries,
                            CHILD_ENTRIES_CACHE_CAP,
                            |key| expanded.contains(key),
                        );
                        this.rebuild_list_entries();
                    }
                    Err(error) => {
                        this.expanded.remove(&path);
                        if this.watched_children.remove(&path) {
                            if let Some(watcher) = this.watcher.as_mut() {
                                let _ = watcher.unwatch(&path);
                            }
                        }
                        this.operation_error =
                            Some(format!("Could not open folder: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn list_folder_key(&mut self, expand: bool, cx: &mut Context<Self>) {
        if self.view != ViewMode::List
            || self.trash_view
            || self.applications_view
            || self.search_summary.is_some()
            || self.selected.len() != 1
        {
            return;
        }
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if entry.is_dir && self.expanded.contains(&entry.path) != expand {
            self.toggle_list_folder(entry.path.clone(), cx);
        }
    }
}

fn append_rows(
    source: &[Entry],
    depth: usize,
    expanded: &BTreeSet<PathBuf>,
    children: &HashMap<PathBuf, Vec<Entry>>,
    rows: &mut Vec<Entry>,
    depths: &mut Vec<usize>,
) {
    for entry in source {
        rows.push(entry.clone());
        depths.push(depth);
        if expanded.contains(&entry.path) {
            if let Some(child_rows) = children.get(&entry.path) {
                append_rows(child_rows, depth + 1, expanded, children, rows, depths);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_rows_keep_parent_order_and_depth() {
        let root = PathBuf::from("/tmp/folder");
        let child = root.join("child");
        let grandchild = child.join("grandchild");
        let make = |path: PathBuf| Entry {
            name: path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
                .into(),
            path,
            is_dir: true,
            size: "".into(),
            modified: "".into(),
            modified_absolute: "".into(),
            created: "".into(),
            created_absolute: "".into(),
            last_opened: "".into(),
            last_opened_absolute: "".into(),
            added: "".into(),
            added_absolute: "".into(),
            kind: "Folder".into(),
            size_bytes: 0,
            mtime: SystemTime::UNIX_EPOCH,
            created_time: SystemTime::UNIX_EPOCH,
            last_opened_time: SystemTime::UNIX_EPOCH,
            tag: None,
            search_detail: None,
            application: None,
        };
        let mut children = HashMap::new();
        children.insert(root.clone(), vec![make(child.clone())]);
        children.insert(child.clone(), vec![make(grandchild.clone())]);
        let mut expanded = BTreeSet::from([root.clone(), child.clone()]);
        let mut rows = Vec::new();
        let mut depths = Vec::new();
        append_rows(
            &[make(root.clone())],
            0,
            &expanded,
            &children,
            &mut rows,
            &mut depths,
        );
        assert_eq!(
            rows.iter().map(|row| &row.path).collect::<Vec<_>>(),
            vec![&root, &child, &grandchild]
        );
        assert_eq!(depths, [0, 1, 2]);

        expanded.remove(&root);
        rows.clear();
        depths.clear();
        append_rows(
            &[make(root.clone())],
            0,
            &expanded,
            &children,
            &mut rows,
            &mut depths,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, root);
    }
}
