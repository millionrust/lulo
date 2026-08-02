use super::*;

impl FinderView {
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
}
