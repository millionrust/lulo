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
        self.entries.push(entry);
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        let Some(index) = self.entries.iter().position(|entry| entry.path == path) else {
            self.reload(cx);
            return;
        };
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
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
            let dst = unique_path_avoiding(self.cwd.join(copy_name), &destinations);
            destinations.insert(dst.clone());
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: src,
                destination: dst,
            });
        }
        self.start_transfer("Duplicating", tasks, false, cx);
    }

    pub(super) fn delete_immediately(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let mut failures = Vec::new();
        for p in self.selected_paths() {
            if let Err(failure) = file_ops::delete(&file_ops::RealFileSystem, &p) {
                failures.push(failure);
            }
        }
        self.finish_file_operations(failures, cx);
    }
}
