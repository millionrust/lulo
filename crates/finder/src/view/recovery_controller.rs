use super::*;

impl FinderView {
    pub(super) fn close_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_busy {
            return;
        }
        self.recovery_open = false;
        cx.notify();
    }

    pub(super) fn resolve_current_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_busy {
            return;
        }
        let Some(review) = self.recovery_reviews.first().cloned() else {
            self.recovery_open = false;
            cx.notify();
            return;
        };
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error =
                Some("File-operation recovery is unavailable; no item was changed".into());
            cx.notify();
            return;
        };
        self.recovery_busy = true;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (outcome, refresh) = cx
                .background_executor()
                .spawn(async move {
                    let outcome = journal.resolve_review(&review);
                    let refresh = journal.recover_unambiguous().and_then(|recovery| {
                        let reviews = journal.review_pending()?;
                        let undo = journal.undo_store().latest()?;
                        Ok((recovery, reviews, undo))
                    });
                    (outcome, refresh)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.recovery_busy = false;
                match refresh {
                    Ok((recovery, reviews, undo)) => {
                        this.pending_operations = reviews.len();
                        this.recovery_reviews = reviews;
                        this.recovery_open = this.pending_operations != 0;
                        this.undo_available = undo;
                        if recovery.finalized != 0 && outcome.is_err() {
                            this.operation_notice = Some(
                                format!(
                                    "Files safely completed {} interrupted file operation{}",
                                    recovery.finalized,
                                    if recovery.finalized == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                    }
                    Err(_) => {
                        this.operation_journal = None;
                        this.undo_available = None;
                        this.pending_operations = 0;
                        this.recovery_reviews.clear();
                        this.recovery_open = false;
                        this.operation_error = Some(
                            "File-operation recovery data could not be verified; transfers are disabled"
                                .into(),
                        );
                        cx.notify();
                        return;
                    }
                }

                match outcome {
                    Ok(operation_journal::ResolutionOutcome::PreservedCopy {
                        complete,
                        name,
                    }) => {
                        this.operation_notice = Some(
                            if complete {
                                format!("Recovery copy preserved as “{name}”")
                            } else {
                                format!(
                                    "Partial recovery copy preserved as “{name}”; inspect it before relying on it"
                                )
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(
                        operation_journal::ResolutionOutcome::PreservedReplacementBackup {
                            complete,
                            name,
                        },
                    ) => {
                        this.operation_notice = Some(
                            if complete {
                                format!("Previous destination preserved as “{name}”")
                            } else {
                                format!(
                                    "Possibly changed previous destination preserved as “{name}”; inspect it before relying on it"
                                )
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(operation_journal::ResolutionOutcome::KeptExistingItems) => {
                        this.operation_notice =
                            Some("Existing items kept; no file was deleted".into());
                        this.operation_error = None;
                    }
                    Err(error) => {
                        this.operation_error = Some(
                            match error.kind() {
                                std::io::ErrorKind::AlreadyExists => {
                                    "The recovery name is no longer available; review the updated choice"
                                }
                                std::io::ErrorKind::WouldBlock => {
                                    "Recovery state changed; review it again before continuing"
                                }
                                _ => {
                                    "Files could not preserve the recovery copy; no existing item was overwritten"
                                }
                            }
                            .into(),
                        );
                        this.recovery_open = !this.recovery_reviews.is_empty();
                    }
                }
                if this.pending_operations != 0 && this.operation_error.is_none() {
                    this.operation_error = Some(
                        format!(
                            "Review {} remaining file operation{} before starting another transfer",
                            this.pending_operations,
                            if this.pending_operations == 1 { "" } else { "s" }
                        )
                        .into(),
                    );
                }
                this.reload(cx);
            });
        })
        .detach();
    }
}
