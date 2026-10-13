use super::*;
use crate::redo::{RedoEntry, RedoKind, RedoStep};

impl FinderView {
    fn current_undo_id(&self) -> Option<&str> {
        self.undo_available
            .as_ref()
            .map(|available| available.id.as_str())
    }

    /// The operation Edit ▸ Redo would repeat right now, if any.
    pub(super) fn redo_ready(&self) -> Option<&RedoEntry> {
        self.redo.current(self.current_undo_id())
    }

    /// Reconcile Redo with the Undo history after a file operation finished
    /// (see `RedoStack::sync`).
    pub(super) fn sync_redo(&mut self) {
        let latest = self.current_undo_id().map(str::to_owned);
        let label = self
            .undo_available
            .as_ref()
            .map(|available| available.label.clone());
        self.redo.sync(latest.as_deref(), label.as_deref());
    }

    /// Remember what a finished Undo reversed so Redo can repeat it.
    pub(super) fn record_redo(&mut self, outcome: &undo_journal::UndoOutcome) {
        let after = self.current_undo_id().map(str::to_owned);
        if let Some(entry) = outcome
            .redo
            .clone()
            .and_then(|steps| RedoEntry::new(steps, outcome.label.clone(), after))
        {
            self.redo.push(entry);
        }
    }

    pub(super) fn start_redo(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.journal_loading {
            self.operation_error = Some("Files is still verifying file-operation recovery".into());
            cx.notify();
            return;
        }
        if self.operation_journal.is_none() {
            self.operation_error =
                Some("File-operation recovery is unavailable; Redo is disabled".into());
            cx.notify();
            return;
        }
        let Some(entry) = self.redo_ready().cloned() else {
            self.operation_notice = Some("There are no undone file operations to redo".into());
            self.operation_error = None;
            cx.notify();
            return;
        };
        let Some(kind) = entry.kind() else {
            return;
        };
        self.operation_error = None;
        self.operation_notice = None;
        let latest = self.current_undo_id().map(str::to_owned);
        self.redo.begin_redo(latest.as_deref());
        match kind {
            RedoKind::Transfer => {
                let moving = entry
                    .steps
                    .iter()
                    .all(|step| matches!(step, RedoStep::Move { .. }));
                let tasks = entry
                    .steps
                    .iter()
                    .filter_map(|step| match step {
                        RedoStep::Move { from, to } => Some(file_ops::TransferTask {
                            kind: file_ops::TransferKind::Move,
                            source: from.clone(),
                            destination: to.clone(),
                        }),
                        RedoStep::Copy { from, to } => Some(file_ops::TransferTask {
                            kind: file_ops::TransferKind::Copy,
                            source: from.clone(),
                            destination: to.clone(),
                        }),
                        _ => None,
                    })
                    .collect();
                let label = if moving { "Moving" } else { "Copying" };
                self.start_transfer_with_conflicts(label, tasks, false, false, cx);
            }
            RedoKind::Trash => {
                let paths = entry
                    .steps
                    .iter()
                    .filter_map(|step| match step {
                        RedoStep::Trash { path } => Some(path.clone()),
                        _ => None,
                    })
                    .collect();
                self.trash_paths(paths, cx);
            }
            RedoKind::NewFolder => {
                let Some(RedoStep::NewFolder { path }) = entry.steps.first().cloned() else {
                    return;
                };
                self.redo_new_folder(path, cx);
            }
        }
    }

    fn redo_new_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(journal) = self.operation_journal.clone() else {
            return;
        };
        self.new_folder_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let folder = path.clone();
            let result = blocking::unblock(move || {
                file_ops::create_folder(&file_ops::RealFileSystem, &folder)
                    .map_err(|error| error.detail)?;
                journal
                    .undo_store()
                    .archive_created_folder(&folder)
                    .map_err(|error| {
                        format!("New folder was created, but Undo could not be recorded: {error}")
                    })?;
                journal
                    .undo_store()
                    .latest()
                    .map_err(|error| error.to_string())
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.new_folder_busy = false;
                match result {
                    Ok(availability) => {
                        this.undo_available = availability;
                        this.sync_redo();
                        this.pending_select = Some(path);
                        this.reload(cx);
                    }
                    Err(error) => this.operation_error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
