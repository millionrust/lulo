use super::*;

impl FinderView {
    // ---- operations ----
    pub(super) fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let path = unique_path(self.cwd.join("untitled folder"));
        if let Err(failure) = file_ops::create_folder(&file_ops::RealFileSystem, &path) {
            self.record_operation_failures(vec![failure], cx);
            return;
        }

        let Some(entry) = entry_for(&path) else {
            self.operation_error = Some("The folder was created but could not be displayed".into());
            self.reload(cx);
            return;
        };
        self.entries.push(entry.clone());
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        let Some(index) = self.entries.iter().position(|entry| entry.path == path) else {
            self.reload(cx);
            return;
        };
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
        if self.view == ViewMode::Column {
            self.column_selection = Some(entry);
        }
        self.operation_error = None;
        self.rename_start(window, cx);
    }

    pub(super) fn duplicate(&mut self, cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        let mut destinations = BTreeSet::new();
        for src in self.selected_paths() {
            let stem = src
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = src.extension().map(|e| e.to_string_lossy().into_owned());
            let copy_name = match &ext {
                Some(e) => format!("{stem} copy.{e}"),
                None => format!("{stem} copy"),
            };
            let destination_dir = src.parent().unwrap_or(self.cwd.as_path());
            let dst = unique_path_avoiding(destination_dir.join(copy_name), &destinations);
            destinations.insert(dst.clone());
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: src,
                destination: dst,
            });
        }
        self.start_transfer("Duplicating", tasks, false, cx);
    }

    /// "Delete Immediately" skips Trash entirely, so — like macOS — it must
    /// never run without the user confirming an unrecoverable delete first.
    /// No confirmation dialog is wired up for this action yet, so it refuses
    /// rather than deleting: silently permitting an unconfirmed, untrashed
    /// delete the moment something (a future keybinding or menu item) calls
    /// this is exactly the trap this guard exists to prevent.
    pub(super) fn delete_immediately(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.selected_paths().is_empty() {
            return;
        }
        self.operation_error =
            Some("Delete Immediately needs a confirmation step that isn't available yet".into());
        cx.notify();
    }
}
