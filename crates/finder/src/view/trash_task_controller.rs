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
    pub(super) fn receive_trash_events(
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
}
