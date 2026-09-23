use super::*;

impl FinderView {
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
            let _ = rmac_sound::play_alert();
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
