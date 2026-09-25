use super::*;

impl FinderView {
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
                                    // Select the item Undo just put back, as
                                    // Finder does (FILES-46).
                                    this.pending_select = outcome.restored_to;
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
}
