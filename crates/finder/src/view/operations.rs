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

    pub(super) fn start_undo(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", test))]
        let trash_busy = self.trash_operation.is_some();
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_busy = false;
        if self.transfer.is_some()
            || self.undo_operation.is_some()
            || trash_busy
            || self.conflict_preflight
            || self.conflict_batch.is_some()
            || self.recovery_busy
        {
            self.operation_error = Some("Wait for the current file operation to finish".into());
            cx.notify();
            return;
        }
        if self.journal_loading {
            self.operation_error = Some("Files is still verifying file-operation recovery".into());
            cx.notify();
            return;
        }
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error =
                Some("File-operation recovery is unavailable; Undo is disabled".into());
            cx.notify();
            return;
        };
        if self.pending_operations != 0 {
            self.operation_error = Some(
                "Review unfinished file operations before undoing a completed operation".into(),
            );
            self.recovery_open = true;
            cx.notify();
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        if self.trash_loading
            || self.trash_pending != 0
            || self.trash_recovery_busy
            || self.trash_recovery_open
        {
            self.operation_error =
                Some("Review unfinished Trash operations before using Undo".into());
            self.trash_recovery_open = self.trash_pending != 0;
            cx.notify();
            return;
        }
        let Some(available) = self.undo_available.clone() else {
            self.operation_notice = Some("There are no completed file operations to undo".into());
            self.operation_error = None;
            cx.notify();
            return;
        };
        #[cfg(any(target_os = "linux", test))]
        let trash_store = self.trash_store.clone();
        #[cfg(any(target_os = "linux", test))]
        if available.uses_trash && trash_store.is_none() {
            self.operation_error =
                Some("Trash recovery is unavailable; this Undo cannot run safely".into());
            cx.notify();
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.operation_error = None;
        self.operation_notice = None;
        self.undo_operation = Some(ActiveUndo {
            label: available.label.into(),
            phase: file_ops::TransferPhase::Scanning,
            bytes_processed: 0,
            cancel: cancel.clone(),
            cancelling: false,
        });
        cx.notify();

        let (events, event_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                let progress_events = events.clone();
                let mut report_progress = |activity| {
                    let _ = progress_events.try_send(UndoEvent::Progress(activity));
                };
                #[cfg(any(target_os = "linux", test))]
                let outcome = match trash_store {
                    Some(store) => store.execute_latest_undo(
                        &file_ops::RealFileSystem,
                        &cancel,
                        &mut report_progress,
                    ),
                    None => journal.undo_store().execute_latest(
                        &file_ops::RealFileSystem,
                        &cancel,
                        &mut report_progress,
                    ),
                };
                #[cfg(not(any(target_os = "linux", test)))]
                let outcome = journal.undo_store().execute_latest(
                    &file_ops::RealFileSystem,
                    &cancel,
                    &mut report_progress,
                );
                let availability = journal.undo_store().latest();
                let _ = events.send_blocking(UndoEvent::Finished {
                    outcome,
                    availability,
                });
            })
            .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = event_rx.recv().await {
                let finished = matches!(event, UndoEvent::Finished { .. });
                if this
                    .update(cx, |this: &mut FinderView, cx| match event {
                        UndoEvent::Progress(activity) => {
                            if let Some(undo) = this.undo_operation.as_mut() {
                                match activity {
                                    file_ops::CopyActivity::Bytes(bytes) => {
                                        undo.phase = file_ops::TransferPhase::Copying;
                                        undo.bytes_processed =
                                            undo.bytes_processed.saturating_add(bytes);
                                    }
                                    file_ops::CopyActivity::Finishing => {
                                        undo.phase = file_ops::TransferPhase::Finishing;
                                    }
                                }
                            }
                            cx.notify();
                        }
                        UndoEvent::Finished {
                            outcome,
                            availability,
                        } => {
                            this.undo_operation = None;
                            match availability {
                                Ok(availability) => this.undo_available = availability,
                                Err(_) => {
                                    this.undo_available = None;
                                    this.operation_journal = None;
                                }
                            }
                            match outcome {
                                Ok(Some(outcome)) => {
                                    this.operation_notice =
                                        Some(format!("{} completed", outcome.label).into());
                                    this.operation_error = None;
                                }
                                Ok(None) => {
                                    this.operation_notice =
                                        Some("There are no completed file operations to undo".into());
                                    this.operation_error = None;
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                    this.operation_notice = Some(
                                        "Undo paused at a durable boundary; press Command-Z to continue"
                                            .into(),
                                    );
                                    this.operation_error = None;
                                }
                                Err(error)
                                    if error.kind() == std::io::ErrorKind::WouldBlock =>
                                {
                                    this.operation_error = Some(
                                        "Undo stopped because an involved item changed; no changed item was removed or replaced"
                                            .into(),
                                    );
                                }
                                Err(error)
                                    if error.kind() == std::io::ErrorKind::AlreadyExists =>
                                {
                                    this.operation_error = Some(
                                        "Undo stopped because the original location is no longer available; no item was overwritten"
                                            .into(),
                                    );
                                }
                                Err(error)
                                    if error.kind() == std::io::ErrorKind::StorageFull =>
                                {
                                    this.operation_error =
                                        Some(format!("Undo needs more free space: {error}").into());
                                }
                                Err(_) => {
                                    this.operation_error = Some(
                                        "Files could not complete Undo; its durable receipt was retained for a safe retry"
                                            .into(),
                                    );
                                }
                            }
                            if this.operation_journal.is_none() {
                                this.operation_error = Some(
                                    "Undo history could not be verified; transfers are disabled until recovery data is repaired"
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

    pub(super) fn cancel_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(undo) = self.undo_operation.as_mut() {
            undo.cancel.store(true, Ordering::Release);
            undo.cancelling = true;
            cx.notify();
        }
    }

    pub(super) fn cancel_trash(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", test))]
        if let Some(operation) = self.trash_operation.as_mut() {
            operation.cancel.store(true, Ordering::Release);
            operation.cancelling = true;
            cx.notify();
        }
        #[cfg(not(any(target_os = "linux", test)))]
        let _ = cx;
    }

    #[cfg(any(target_os = "linux", test))]
    fn receive_trash_events(
        &mut self,
        event_rx: async_channel::Receiver<TrashEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = event_rx.recv().await {
                let finished = matches!(event, TrashEvent::Finished(_));
                if this
                    .update(cx, |this: &mut FinderView, cx| match event {
                        TrashEvent::Progress { processed, total } => {
                            if let Some(operation) = this.trash_operation.as_mut() {
                                operation.processed = processed;
                                operation.total = total;
                            }
                            cx.notify();
                        }
                        TrashEvent::Finished(completion) => this.finish_trash_task(completion, cx),
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

    #[cfg(any(target_os = "linux", test))]
    fn finish_trash_task(&mut self, completion: TrashCompletion, cx: &mut Context<Self>) {
        let TrashCompletion {
            kind,
            completed,
            cancelled,
            failures,
            recovery,
            undo_availability,
        } = completion;
        self.trash_operation = None;
        match undo_availability {
            Ok(availability) => self.undo_available = availability,
            Err(_) => {
                self.undo_available = None;
                self.operation_journal = None;
            }
        }
        let recovery_unavailable = match recovery {
            Ok((recovery, reviews)) => {
                self.trash_pending = recovery.pending;
                self.trash_recovery_reviews = reviews;
                self.trash_recovery_open = recovery.pending != 0;
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                self.operation_notice =
                    Some("Another Files window is safely handling Trash".into());
                false
            }
            Err(_) => {
                self.trash_store = None;
                self.trash_pending = 0;
                self.trash_recovery_reviews.clear();
                self.trash_recovery_open = false;
                self.operation_error = Some(
                    "Trash recovery data could not be verified; Trash actions are disabled".into(),
                );
                true
            }
        };
        if !failures.is_empty() {
            self.operation_notice = None;
            self.record_operation_failures(failures, cx);
        }
        if recovery_unavailable {
            let unavailable = "Trash recovery is unavailable; Trash actions are disabled";
            self.operation_error = Some(
                match self.operation_error.take() {
                    Some(failure) => format!("{failure}. {unavailable}"),
                    None => unavailable.to_string(),
                }
                .into(),
            );
        }
        if self.trash_pending != 0 {
            let retained = format!(
                "{} changed Trash operation{} retained for manual recovery",
                self.trash_pending,
                if self.trash_pending == 1 {
                    " was"
                } else {
                    "s were"
                }
            );
            self.operation_error = Some(
                match self.operation_error.take() {
                    Some(failure) => format!("{failure}. {retained}"),
                    None => retained,
                }
                .into(),
            );
        } else if cancelled && self.operation_error.is_none() {
            self.operation_notice = Some(
                match (kind, completed) {
                    (TrashTaskKind::Move, 0) => {
                        "Move to Trash cancelled; no item was moved".to_string()
                    }
                    (TrashTaskKind::Move, completed) => format!(
                        "Move to Trash cancelled after moving {completed} item{}",
                        if completed == 1 { "" } else { "s" }
                    ),
                    (TrashTaskKind::Restore, 0) => {
                        "Restore cancelled; no item was restored".to_string()
                    }
                    (TrashTaskKind::Restore, completed) => format!(
                        "Restore cancelled after restoring {completed} item{}",
                        if completed == 1 { "" } else { "s" }
                    ),
                    (TrashTaskKind::Delete, 0) => {
                        "Permanent deletion cancelled; no item was deleted".to_string()
                    }
                    (TrashTaskKind::Delete, completed) => format!(
                        "Permanent deletion cancelled after deleting {completed} item{}",
                        if completed == 1 { "" } else { "s" }
                    ),
                }
                .into(),
            );
        } else if self.operation_error.is_none() {
            self.operation_notice = Some(
                match (kind, completed) {
                    (TrashTaskKind::Move, 1) => "Moved 1 item to Trash".to_string(),
                    (TrashTaskKind::Move, completed) => {
                        format!("Moved {completed} items to Trash")
                    }
                    (TrashTaskKind::Restore, 1) => "Restored 1 item".to_string(),
                    (TrashTaskKind::Restore, completed) => {
                        format!("Restored {completed} items")
                    }
                    (TrashTaskKind::Delete, 1) => "Permanently deleted 1 item".to_string(),
                    (TrashTaskKind::Delete, completed) => {
                        format!("Permanently deleted {completed} items")
                    }
                }
                .into(),
            );
        }
        self.reload(cx);
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn close_trash_recovery(&mut self, cx: &mut Context<Self>) {
        if self.trash_recovery_busy {
            return;
        }
        self.trash_recovery_open = false;
        cx.notify();
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn resolve_current_trash_recovery(&mut self, cx: &mut Context<Self>) {
        if self.trash_recovery_busy {
            return;
        }
        let Some(review) = self.trash_recovery_reviews.first().cloned() else {
            self.trash_recovery_open = false;
            cx.notify();
            return;
        };
        let Some(store) = self.trash_store.clone() else {
            self.operation_error =
                Some("Trash recovery is unavailable; no item was changed".into());
            cx.notify();
            return;
        };
        self.trash_recovery_busy = true;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (outcome, refresh) = cx
                .background_executor()
                .spawn(async move {
                    match store.resolve_review_and_refresh(&review) {
                        Ok((outcome, recovery, reviews, undo)) => {
                            (Ok(outcome), Ok((recovery, reviews, undo)))
                        }
                        Err(error) => {
                            let refresh =
                                store.recover_and_review().and_then(|(recovery, reviews)| {
                                    let undo = store.undo_store().latest()?;
                                    Ok((recovery, reviews, undo))
                                });
                            (Err(error), refresh)
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.trash_recovery_busy = false;
                match refresh {
                    Ok((recovery, reviews, undo)) => {
                        this.trash_pending = recovery.pending;
                        this.trash_recovery_reviews = reviews;
                        this.trash_recovery_open = recovery.pending != 0;
                        this.undo_available = undo;
                    }
                    Err(_) => {
                        this.trash_store = None;
                        this.trash_pending = 0;
                        this.trash_recovery_reviews.clear();
                        this.trash_recovery_open = false;
                        this.operation_error = Some(
                            "Trash recovery data could not be verified; Trash actions are disabled"
                                .into(),
                        );
                        cx.notify();
                        return;
                    }
                }

                match outcome {
                    Ok(trash_store::TrashResolutionOutcome::ReturnedRemainingItem {
                        may_be_partial,
                    }) => {
                        this.operation_notice = Some(
                            if may_be_partial {
                                "Remaining data returned to Trash; it may be incomplete"
                            } else {
                                "Item returned safely to Trash"
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(
                        trash_store::TrashResolutionOutcome::ReturnedRemainingItemAndRebuiltMetadata {
                            may_be_partial,
                        },
                    ) => {
                        this.operation_notice = Some(
                            if may_be_partial {
                                "Remaining data returned to Trash with rebuilt metadata; it may be incomplete"
                            } else {
                                "Item returned to Trash with rebuilt metadata"
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(
                        trash_store::TrashResolutionOutcome::PreservedConflictingItems {
                            rebuilt_metadata,
                        },
                    ) => {
                        this.operation_notice = Some(
                            if rebuilt_metadata {
                                "Both Trash copies kept under separate names; missing metadata was rebuilt"
                            } else {
                                "Both Trash copies kept safely under separate names"
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(trash_store::TrashResolutionOutcome::RebuiltMetadata) => {
                        this.operation_notice =
                            Some("Trash metadata rebuilt without changing the item".into());
                        this.operation_error = None;
                    }
                    Ok(trash_store::TrashResolutionOutcome::RemovedOrphanMetadata) => {
                        this.operation_notice =
                            Some("Orphaned Trash metadata removed; no user file was deleted".into());
                        this.operation_error = None;
                    }
                    Ok(trash_store::TrashResolutionOutcome::KeptExistingItems) => {
                        this.operation_notice = Some(
                            "Existing items kept; only the exact recovery record was cleared"
                                .into(),
                        );
                        this.operation_error = None;
                    }
                    Err(error) => {
                        this.operation_error = Some(
                            match error.kind() {
                                std::io::ErrorKind::AlreadyExists => {
                                    "The Trash item location is no longer available; review the updated state"
                                }
                                std::io::ErrorKind::WouldBlock => {
                                    "Trash recovery changed; review it again before continuing"
                                }
                                _ => {
                                    "Files could not resolve Trash recovery; no existing item was replaced"
                                }
                            }
                            .into(),
                        );
                        this.trash_recovery_open = !this.trash_recovery_reviews.is_empty();
                    }
                }
                if this.trash_pending != 0 && this.operation_error.is_none() {
                    this.operation_error = Some(
                        format!(
                            "Review {} remaining Trash operation{} before using Trash",
                            this.trash_pending,
                            if this.trash_pending == 1 { "" } else { "s" }
                        )
                        .into(),
                    );
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    // ---- operations ----
    pub(super) fn new_folder(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let path = unique_path(self.cwd.join("untitled folder"));
        let failures = file_ops::create_folder(&file_ops::RealFileSystem, &path)
            .err()
            .into_iter()
            .collect();
        self.finish_file_operations(failures, cx);
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

    pub(super) fn move_to_trash(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.restore_selected(cx);
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        #[cfg(any(target_os = "linux", test))]
        {
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            let Some(store) = self.trash_store.clone() else {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            };
            if self.trash_pending != 0 {
                self.operation_error = Some(
                    "A changed Trash operation needs manual recovery before another item can be moved"
                        .into(),
                );
                cx.notify();
                return;
            }
            let total = paths.len();
            let cancel = Arc::new(AtomicBool::new(false));
            self.trash_operation = Some(ActiveTrash {
                label: "Moving to Trash".into(),
                processed: 0,
                total,
                cancel: cancel.clone(),
                cancelling: false,
            });
            self.operation_error = None;
            self.operation_notice = None;
            cx.notify();

            let (events, event_rx) = async_channel::bounded(16);
            cx.background_executor()
                .spawn(async move {
                    let mut failures = Vec::new();
                    let mut completed = 0usize;
                    let mut processed = 0usize;
                    let mut cancelled = false;
                    for path in paths {
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        match store.trash(&path, &cancel) {
                            Ok(()) => completed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                cancelled = true;
                                break;
                            }
                            Err(error) => {
                                let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                                failures.push(file_ops::Failure::message(
                                    file_ops::Operation::Trash,
                                    &path,
                                    None,
                                    error.to_string(),
                                ));
                                if blocked {
                                    processed += 1;
                                    let _ =
                                        events.try_send(TrashEvent::Progress { processed, total });
                                    break;
                                }
                            }
                        }
                        processed += 1;
                        let _ = events.try_send(TrashEvent::Progress { processed, total });
                    }
                    let recovery = store.recover_and_review();
                    let undo_availability = store.undo_store().latest();
                    let _ = events.send_blocking(TrashEvent::Finished(TrashCompletion {
                        kind: TrashTaskKind::Move,
                        completed,
                        cancelled,
                        failures,
                        recovery,
                        undo_availability,
                    }));
                })
                .detach();
            self.receive_trash_events(event_rx, cx);
        }

        #[cfg(not(any(target_os = "linux", test)))]
        {
            let failures = trash::delete_all(&paths)
                .err()
                .map(|error| {
                    file_ops::Failure::message(
                        file_ops::Operation::Trash,
                        &paths[0],
                        None,
                        error.to_string(),
                    )
                })
                .into_iter()
                .collect();
            self.finish_file_operations(failures, cx);
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn selected_trash_items(&self) -> Vec<trash_store::TrashedItem> {
        let selected_paths = self.selected_paths().into_iter().collect::<BTreeSet<_>>();
        self.trash_items
            .iter()
            .filter(|item| selected_paths.contains(item.data_path()))
            .cloned()
            .collect()
    }

    pub(super) fn restore_selected(&mut self, cx: &mut Context<Self>) {
        if !self.trash_view {
            self.operation_error = Some("Open Trash to restore items".into());
            cx.notify();
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            if self.transfer.is_some()
                || self.undo_operation.is_some()
                || self.trash_operation.is_some()
            {
                self.operation_error = Some("Wait for the current file operation to finish".into());
                cx.notify();
                return;
            }
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            let Some(store) = self.trash_store.clone() else {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            };
            if self.trash_pending != 0 {
                self.operation_error =
                    Some("A changed Trash operation needs manual recovery before restore".into());
                cx.notify();
                return;
            }
            let items = self.selected_trash_items();
            if items.is_empty() {
                return;
            }
            let total = items.len();
            let cancel = Arc::new(AtomicBool::new(false));
            self.trash_operation = Some(ActiveTrash {
                label: "Restoring".into(),
                processed: 0,
                total,
                cancel: cancel.clone(),
                cancelling: false,
            });
            self.operation_error = None;
            self.operation_notice = None;
            cx.notify();

            let (events, event_rx) = async_channel::bounded(16);
            cx.background_executor()
                .spawn(async move {
                    let mut failures = Vec::new();
                    let mut completed = 0usize;
                    let mut processed = 0usize;
                    let mut cancelled = false;
                    for item in items {
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        match store.restore(&item, &cancel) {
                            Ok(_) => completed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                cancelled = true;
                                break;
                            }
                            Err(error) => {
                                let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                                failures.push(file_ops::Failure::message(
                                    file_ops::Operation::Restore,
                                    &item.original_path,
                                    None,
                                    error.to_string(),
                                ));
                                if blocked {
                                    processed += 1;
                                    let _ =
                                        events.try_send(TrashEvent::Progress { processed, total });
                                    break;
                                }
                            }
                        }
                        processed += 1;
                        let _ = events.try_send(TrashEvent::Progress { processed, total });
                    }
                    let recovery = store.recover_and_review();
                    let undo_availability = store.undo_store().latest();
                    let _ = events.send_blocking(TrashEvent::Finished(TrashCompletion {
                        kind: TrashTaskKind::Restore,
                        completed,
                        cancelled,
                        failures,
                        recovery,
                        undo_availability,
                    }));
                })
                .detach();
            self.receive_trash_events(event_rx, cx);
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.operation_error = Some("Trash restore is available on Linux".into());
            cx.notify();
        }
    }

    pub(super) fn request_permanent_delete(&mut self, cx: &mut Context<Self>) {
        if !self.trash_view {
            self.operation_error =
                Some("Permanent deletion is available for items in Trash".into());
            cx.notify();
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            if self.transfer.is_some()
                || self.undo_operation.is_some()
                || self.trash_operation.is_some()
                || self.recovery_open
                || self.recovery_busy
                || self.trash_recovery_open
                || self.trash_recovery_busy
            {
                self.operation_error = Some("Wait for the current file operation to finish".into());
                cx.notify();
                return;
            }
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            if self.trash_store.is_none() {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            }
            if self.trash_pending != 0 {
                self.operation_error = Some(
                    "A changed Trash operation needs manual recovery before permanent deletion"
                        .into(),
                );
                cx.notify();
                return;
            }
            let items = self.selected_trash_items();
            if items.is_empty() {
                return;
            }
            self.menu_at = None;
            self.operation_error = None;
            self.operation_notice = None;
            self.delete_confirmation = Some(DeleteConfirmation { items });
            cx.notify();
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.operation_error = Some("Permanent deletion is available on Linux".into());
            cx.notify();
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn cancel_permanent_delete(&mut self, cx: &mut Context<Self>) {
        self.delete_confirmation = None;
        cx.notify();
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn confirm_permanent_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirmation) = self.delete_confirmation.take() else {
            return;
        };
        let Some(store) = self.trash_store.clone() else {
            self.operation_error =
                Some("Trash recovery is unavailable; no item was changed".into());
            cx.notify();
            return;
        };
        if self.transfer.is_some()
            || self.undo_operation.is_some()
            || self.trash_operation.is_some()
            || self.trash_pending != 0
        {
            self.operation_error = Some("Wait for the current file operation to finish".into());
            cx.notify();
            return;
        }
        let items = confirmation.items;
        let total = items.len();
        if total == 0 {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.trash_operation = Some(ActiveTrash {
            label: "Deleting Permanently".into(),
            processed: 0,
            total,
            cancel: cancel.clone(),
            cancelling: false,
        });
        self.operation_error = None;
        self.operation_notice = None;
        cx.notify();

        let (events, event_rx) = async_channel::bounded(16);
        cx.background_executor()
            .spawn(async move {
                let mut failures = Vec::new();
                let mut completed = 0usize;
                let mut processed = 0usize;
                let mut cancelled = false;
                for item in items {
                    if cancel.load(Ordering::Acquire) {
                        cancelled = true;
                        break;
                    }
                    match store.delete_permanently(&item, &cancel) {
                        Ok(()) => completed += 1,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                            cancelled = true;
                            break;
                        }
                        Err(error) => {
                            let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                            failures.push(file_ops::Failure::message(
                                file_ops::Operation::PermanentDelete,
                                &item.original_path,
                                None,
                                error.to_string(),
                            ));
                            if blocked {
                                processed += 1;
                                let _ = events.try_send(TrashEvent::Progress { processed, total });
                                break;
                            }
                        }
                    }
                    processed += 1;
                    let _ = events.try_send(TrashEvent::Progress { processed, total });
                }
                let recovery = store.recover_and_review();
                let undo_availability = store.undo_store().latest();
                let _ = events.send_blocking(TrashEvent::Finished(TrashCompletion {
                    kind: TrashTaskKind::Delete,
                    completed,
                    cancelled,
                    failures,
                    recovery,
                    undo_availability,
                }));
            })
            .detach();
        self.receive_trash_events(event_rx, cx);
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
