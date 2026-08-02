use super::*;

impl FinderView {
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
}
