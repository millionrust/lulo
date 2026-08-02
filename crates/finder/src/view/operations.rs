use super::*;

impl FinderView {
    pub(super) fn block_mutation_during_transfer(&mut self, cx: &mut Context<Self>) -> bool {
        if self.trash_view {
            self.operation_error =
                Some("Use Restore for items in Trash; direct changes are disabled".into());
            cx.notify();
            return true;
        }
        #[cfg(any(target_os = "linux", test))]
        let trash_busy = self.trash_operation.is_some();
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_busy = false;
        if self.transfer.is_none()
            && self.undo_operation.is_none()
            && !trash_busy
            && !self.conflict_preflight
            && self.conflict_batch.is_none()
        {
            return false;
        }
        self.operation_error = Some("Wait for the current file operation to finish".into());
        cx.notify();
        true
    }

    pub(super) fn start_transfer_with_conflicts(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
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
            let prepared =
                cx.background_executor()
                    .spawn(async move {
                        prepare_conflict_batch(label, tasks, keep_unfinished_in_clipboard)
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

    fn start_transfer(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        cx: &mut Context<Self>,
    ) {
        self.start_transfer_with_retained(
            label,
            tasks,
            keep_unfinished_in_clipboard,
            Vec::new(),
            cx,
        );
    }

    fn start_transfer_with_retained(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        retained_clipboard: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if tasks.is_empty() {
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.journal_loading {
            self.operation_error = Some("Files is still verifying file-operation recovery".into());
            cx.notify();
            return;
        }
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error =
                Some("File-operation recovery is unavailable; transfers are disabled".into());
            cx.notify();
            return;
        };
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

        let cancel = Arc::new(AtomicBool::new(false));
        self.operation_error = None;
        self.transfer = Some(ActiveTransfer {
            label: label.into(),
            phase: file_ops::TransferPhase::Scanning,
            processed: 0,
            total: tasks.len(),
            bytes_processed: 0,
            bytes_total: 0,
            cancel: cancel.clone(),
            cancelling: false,
            keep_unfinished_in_clipboard,
            retained_clipboard,
        });
        cx.notify();

        // Byte progress can advance faster than the renderer. Keep this bridge
        // bounded and drop intermediate snapshots; the terminal result still
        // uses backpressure and is never intentionally discarded.
        let (events, event_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                let progress_events = events.clone();
                let report = file_ops::execute_transfers(
                    &file_ops::RealFileSystem,
                    Some(journal.as_ref()),
                    &tasks,
                    &cancel,
                    move |progress| {
                        let _ = progress_events.try_send(TransferEvent::Progress(progress));
                    },
                );
                let recovery_reviews = journal
                    .recover_unambiguous()
                    .and_then(|_| journal.review_pending());
                let undo_availability = journal.undo_store().latest();
                let _ = events.send_blocking(TransferEvent::Finished {
                    report,
                    recovery_reviews,
                    undo_availability,
                });
            })
            .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = event_rx.recv().await {
                let finished = matches!(event, TransferEvent::Finished { .. });
                if this
                    .update(cx, |this: &mut FinderView, cx| match event {
                        TransferEvent::Progress(progress) => {
                            if let Some(transfer) = this.transfer.as_mut() {
                                transfer.phase = progress.phase;
                                transfer.processed = progress.processed;
                                transfer.total = progress.total;
                                transfer.bytes_processed = progress.bytes_processed;
                                transfer.bytes_total = progress.bytes_total;
                            }
                            cx.notify();
                        }
                        TransferEvent::Finished {
                            report,
                            recovery_reviews,
                            undo_availability,
                        } => {
                            let keep_clipboard = this
                                .transfer
                                .as_ref()
                                .is_some_and(|transfer| transfer.keep_unfinished_in_clipboard);
                            let retained_clipboard = this
                                .transfer
                                .as_ref()
                                .map(|transfer| transfer.retained_clipboard.clone())
                                .unwrap_or_default();
                            this.transfer = None;
                            if keep_clipboard {
                                let mut unfinished = retained_clipboard;
                                unfinished.extend(report.unfinished_moves);
                                unfinished.sort();
                                unfinished.dedup();
                                this.clipboard = unfinished;
                                this.clip_cut = !this.clipboard.is_empty();
                                if this.clip_cut {
                                    this.write_clip_text(cx);
                                }
                            }
                            match recovery_reviews {
                                Ok(reviews) => {
                                    this.pending_operations = reviews.len();
                                    this.recovery_open = !reviews.is_empty();
                                    this.recovery_reviews = reviews;
                                }
                                Err(_) => {
                                    this.operation_journal = None;
                                    this.pending_operations = 0;
                                    this.recovery_reviews.clear();
                                    this.recovery_open = false;
                                }
                            }
                            match undo_availability {
                                Ok(availability) => this.undo_available = availability,
                                Err(_) => {
                                    this.undo_available = None;
                                    this.operation_journal = None;
                                }
                            }
                            if this.operation_journal.is_none() {
                                this.undo_available = None;
                            }
                            this.record_operation_failures(report.failures, cx);
                            if this.operation_journal.is_none() {
                                this.operation_error = Some(
                                    "File-operation recovery data could not be verified; transfers are disabled"
                                        .into(),
                                );
                            } else if this.pending_operations != 0
                                && this.operation_error.is_none()
                            {
                                this.operation_error = Some(
                                    format!(
                                        "Files retained {} unfinished file-operation record{} for recovery",
                                        this.pending_operations,
                                        if this.pending_operations == 1 { "" } else { "s" }
                                    )
                                    .into(),
                                );
                            }
                            this.reload(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn cancel_transfer(&mut self, cx: &mut Context<Self>) {
        if let Some(transfer) = self.transfer.as_mut() {
            transfer.cancel.store(true, Ordering::Release);
            transfer.cancelling = true;
            cx.notify();
        }
    }
}
