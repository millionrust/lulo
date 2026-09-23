use super::*;

impl FinderView {
    pub(super) fn start_transfer_with_conflicts(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        play_drop_sound: bool,
        cx: &mut Context<Self>,
    ) {
        if tasks.is_empty() || self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.journal_loading {
            self.operation_error = Some("Files is still verifying file-operation recovery".into());
            cx.notify();
            return;
        }
        if self.operation_journal.is_none() {
            self.operation_error =
                Some("File-operation recovery is unavailable; transfers are disabled".into());
            cx.notify();
            return;
        }
        if self.pending_operations != 0 {
            self.operation_error = Some(
                format!(
                    "Resolve {} unfinished file operation{} before starting another transfer",
                    self.pending_operations,
                    if self.pending_operations == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
                .into(),
            );
            self.recovery_open = true;
            cx.notify();
            return;
        }

        self.conflict_preflight = true;
        self.operation_error = None;
        self.operation_notice = Some("Checking for file-name conflicts…".into());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    prepare_conflict_batch(
                        label,
                        tasks,
                        keep_unfinished_in_clipboard,
                        play_drop_sound,
                    )
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.conflict_preflight = false;
                this.operation_notice = None;
                match prepared {
                    Ok(batch) if batch.conflicts.is_empty() => {
                        this.start_transfer_with_retained(
                            batch.label,
                            batch.ready,
                            batch.keep_unfinished_in_clipboard,
                            batch.skipped_moves,
                            batch.play_drop_sound,
                            cx,
                        );
                    }
                    Ok(batch) => {
                        this.conflict_batch = Some(batch);
                        this.conflict_busy = false;
                        cx.notify();
                    }
                    Err(_) => {
                        this.operation_error = Some(
                            "Files could not verify the conflicting items; nothing was changed"
                                .into(),
                        );
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn resolve_current_conflict(
        &mut self,
        decision: ConflictDecision,
        cx: &mut Context<Self>,
    ) {
        if self.conflict_busy {
            return;
        }
        let Some(batch) = self.conflict_batch.as_ref() else {
            return;
        };
        let Some(conflict) = batch.conflicts.front().cloned() else {
            return;
        };
        let reserved = batch.reserved_destinations.clone();
        if decision == ConflictDecision::Replace
            && (conflict.destination_snapshot.is_none() || conflict.source == conflict.destination)
        {
            return;
        }

        self.conflict_busy = true;
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    resolve_conflict_task(&conflict, decision, &reserved)
                        .map(|task| (task, conflict))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.conflict_busy = false;
                let (task, resolved) = match result {
                    Ok(result) => result,
                    Err(_) => {
                        this.conflict_batch = None;
                        this.operation_error = Some(
                            "The source or destination changed while the conflict was open; nothing was changed. Start the operation again to review the current items."
                                .into(),
                        );
                        cx.notify();
                        return;
                    }
                };
                let Some(batch) = this.conflict_batch.as_mut() else {
                    return;
                };
                batch.conflicts.pop_front();
                if decision == ConflictDecision::Skip
                    && resolved.kind == ConflictTransferKind::Move
                    && batch.keep_unfinished_in_clipboard
                {
                    batch.skipped_moves.push(resolved.source);
                }
                if let Some(task) = task {
                    batch.reserved_destinations.insert(task.destination.clone());
                    batch.ready.push(task);
                }
                if batch.conflicts.is_empty() {
                    let batch = this
                        .conflict_batch
                        .take()
                        .expect("completed conflict batch should still exist");
                    if batch.ready.is_empty() {
                        if batch.keep_unfinished_in_clipboard {
                            this.clipboard = batch.skipped_moves;
                            this.clipboard.sort();
                            this.clipboard.dedup();
                            this.clip_cut = !this.clipboard.is_empty();
                            if this.clip_cut {
                                this.write_clip_text(cx);
                            }
                        }
                        this.operation_notice =
                            Some("Skipped the conflicting items; nothing was changed".into());
                        cx.notify();
                    } else {
                        this.start_transfer_with_retained(
                            batch.label,
                            batch.ready,
                            batch.keep_unfinished_in_clipboard,
                            batch.skipped_moves,
                            batch.play_drop_sound,
                            cx,
                        );
                    }
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
