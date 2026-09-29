use super::*;

impl FinderView {
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
                        this.child_entries.insert(path, entries);
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
            kind: "Folder".into(),
            size_bytes: 0,
            mtime: SystemTime::UNIX_EPOCH,
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
