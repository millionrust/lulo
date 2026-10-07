use super::*;

impl FinderView {
    pub(super) fn cancel_trash(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", all(test, unix)))]
        if let Some(operation) = self.trash_operation.as_mut() {
            operation.cancel.store(true, Ordering::Release);
            operation.cancelling = true;
            cx.notify();
        }
        #[cfg(not(any(target_os = "linux", all(test, unix))))]
        let _ = cx;
    }

    #[cfg(any(target_os = "linux", all(test, unix)))]
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

    #[cfg(any(target_os = "linux", all(test, unix)))]
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
        // A housekeeping check runs after every Trash task to verify the
        // journal; it is independent of whether the requested items were
        // actually moved/restored/deleted. When at least one item did
        // complete, a hiccup in that *separate* check must not be reported
        // as "Trash isn't available" — Trash plainly just worked. Future
        // actions still get disabled below for safety; only the alarming,
        // misleading wording is held back while something already succeeded.
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
                if completed == 0 {
                    self.operation_error = Some(
                        "Trash recovery data could not be verified; Trash actions are disabled"
                            .into(),
                    );
                }
                true
            }
        };
        if !failures.is_empty() {
            self.operation_notice = None;
            self.record_operation_failures(failures, cx);
        }
        if recovery_unavailable && completed == 0 {
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
                "{} changed {} operation{} retained for manual recovery",
                self.trash_pending,
                self.file_words.bin(),
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
                        format!(
                            "Move to {} cancelled; no item was moved",
                            self.file_words.bin()
                        )
                    }
                    (TrashTaskKind::Move, completed) => format!(
                        "Move to {} cancelled after moving {completed} item{}",
                        self.file_words.bin(),
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
            self.operation_notice =
                Some(trash_task_summary(kind, completed, recovery_unavailable).into());
        }
        self.reload(cx);
    }
}

/// The notice shown after a Trash task that did not fail outright: what
/// happened, plus (if the post-task housekeeping check could not verify the
/// journal) a calm follow-up — never the alarming "Trash isn't available"
/// wording, since the task itself just demonstrably worked.
#[cfg(any(target_os = "linux", test))]
fn trash_task_summary(kind: TrashTaskKind, completed: usize, recovery_unavailable: bool) -> String {
    let mut notice = match (kind, completed) {
        (TrashTaskKind::Move, 1) => "Moved 1 item to Trash".to_string(),
        (TrashTaskKind::Move, completed) => format!("Moved {completed} items to Trash"),
        (TrashTaskKind::Restore, 1) => "Restored 1 item".to_string(),
        (TrashTaskKind::Restore, completed) => format!("Restored {completed} items"),
        (TrashTaskKind::Delete, 1) => "Permanently deleted 1 item".to_string(),
        (TrashTaskKind::Delete, completed) => format!("Permanently deleted {completed} items"),
    };
    if recovery_unavailable {
        notice.push_str(". Trash needs attention before more changes");
    }
    notice
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_task_summary_reports_what_happened() {
        assert_eq!(
            trash_task_summary(TrashTaskKind::Move, 1, false),
            "Moved 1 item to Trash"
        );
        assert_eq!(
            trash_task_summary(TrashTaskKind::Move, 3, false),
            "Moved 3 items to Trash"
        );
        assert_eq!(
            trash_task_summary(TrashTaskKind::Restore, 1, false),
            "Restored 1 item"
        );
        assert_eq!(
            trash_task_summary(TrashTaskKind::Delete, 2, false),
            "Permanently deleted 2 items"
        );
    }

    /// A move that actually completed must keep saying so, even when the
    /// housekeeping check that ran right after it could not verify the
    /// journal — this is the exact case that used to show the misleading
    /// "Trash isn't available right now—try again." banner despite the
    /// item having already arrived in Trash.
    #[test]
    fn trash_task_summary_adds_a_calm_follow_up_instead_of_hiding_success() {
        let summary = trash_task_summary(TrashTaskKind::Move, 1, true);
        assert!(summary.starts_with("Moved 1 item to Trash"));
        assert!(summary.contains("needs attention"));
        assert!(!summary.to_ascii_lowercase().contains("isn't available"));
    }
}
