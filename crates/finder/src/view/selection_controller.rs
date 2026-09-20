use super::*;

impl FinderView {
    pub(super) fn select_single(&mut self, index: usize) {
        self.column_selection = None;
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
    }

    pub(super) fn handle_click(&mut self, index: usize, command: bool, shift: bool) {
        self.column_selection = None;
        if command {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(index);
        } else if shift {
            if let Some(anchor) = self.anchor {
                let (low, high) = if anchor <= index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                self.selected.clear();
                for selected in low..=high {
                    self.selected.insert(selected);
                }
            } else {
                self.select_single(index);
            }
        } else {
            self.select_single(index);
        }
    }

    pub(super) fn selected_paths(&self) -> Vec<PathBuf> {
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            return self
                .column_selection
                .as_ref()
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default();
        }
        self.selected
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            return self.column_selection.as_ref();
        }
        self.selected
            .iter()
            .next()
            .and_then(|index| self.entries.get(*index))
    }

    pub(super) fn selection_count(&self) -> usize {
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            usize::from(self.column_selection.is_some())
        } else {
            self.selected.len()
        }
    }

    pub(super) fn write_clip_text(&self, cx: &mut Context<Self>) {
        pasteboard::write_file_urls(&self.clipboard);
        let text = self
            .clipboard
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if self.applications_view {
            self.operation_error = Some("Applications cannot be copied from this view".into());
            cx.notify();
            return;
        }
        if self.trash_view {
            self.operation_error = Some("Restore items before copying them".into());
            cx.notify();
            return;
        }
        self.clipboard = self.selected_paths();
        self.clip_cut = false;
        self.write_clip_text(cx);
    }

    pub(super) fn cut(&mut self, cx: &mut Context<Self>) {
        if self.applications_view {
            self.operation_error = Some("Applications cannot be moved from this view".into());
            cx.notify();
            return;
        }
        if self.trash_view {
            self.operation_error = Some("Use Restore to move an item out of Trash".into());
            cx.notify();
            return;
        }
        self.clipboard = self.selected_paths();
        self.clip_cut = true;
        self.write_clip_text(cx);
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.clipboard.is_empty() {
            let mut paths = pasteboard::read_file_urls();
            paths.retain(|path| path.exists());
            if paths.is_empty() {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    paths = text
                        .lines()
                        .map(PathBuf::from)
                        .filter(|path| path.exists())
                        .collect();
                }
            }
            if !paths.is_empty() {
                self.clipboard = paths;
                self.clip_cut = false;
            }
        }
        let kind = if self.clip_cut {
            file_ops::TransferKind::Move
        } else {
            file_ops::TransferKind::Copy
        };
        let mut tasks = Vec::new();
        for source in self.clipboard.clone() {
            if self.clip_cut && source.parent() == Some(self.cwd.as_path()) {
                continue;
            }
            let name = source
                .file_name()
                .map(|name| name.to_owned())
                .unwrap_or_default();
            tasks.push(file_ops::TransferTask {
                kind: kind.clone(),
                source,
                destination: self.cwd.join(name),
            });
        }
        if tasks.is_empty() {
            if self.clip_cut {
                self.clipboard.clear();
                self.clip_cut = false;
                pasteboard::clear_file_urls();
                self.operation_notice =
                    Some("The items are already in this folder; nothing was moved".into());
                cx.notify();
            }
            return;
        }
        self.start_transfer_with_conflicts(
            if self.clip_cut { "Moving" } else { "Copying" },
            tasks,
            self.clip_cut,
            cx,
        );
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        let query = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
        self.selected = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        cx.notify();
    }

    pub(super) fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.reload(cx);
    }

    pub(super) fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        self.search_relevance_order = false;
        self.selected.clear();
        cx.notify();
    }
}
