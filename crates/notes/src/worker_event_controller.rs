use super::*;

impl NotesView {
    pub(super) fn apply_worker_event_with(
        &mut self,
        event: WorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
        after_session_apply: impl FnOnce(&mut Self),
    ) {
        let accepted_generation = match &event {
            WorkerEvent::Accepted(accepted) => accepted.generation,
            _ => None,
        };
        let refresh_search = matches!(&event, WorkerEvent::Ready(_) | WorkerEvent::Accepted(_));
        let sync_editor = match &event {
            WorkerEvent::Ready(_) => self.latest_local_generation.is_none(),
            WorkerEvent::Accepted(accepted) => match accepted.generation {
                Some(generation) => self
                    .latest_local_generation
                    .is_none_or(|latest| generation >= latest),
                None => self.latest_local_generation.is_none(),
            },
            _ => false,
        };
        let restored_draft = match &event {
            WorkerEvent::DraftRestored(restored) => Some(restored.clone()),
            _ => None,
        };
        let accepted_request_id = match &event {
            WorkerEvent::Accepted(accepted) => Some(accepted.request_id),
            _ => None,
        };
        let rejected_request_id = match &event {
            WorkerEvent::Rejected(rejected) => Some(rejected.request_id),
            _ => None,
        };
        let exported = match &event {
            WorkerEvent::Exported(exported) => Some(*exported),
            _ => None,
        };
        let worker_ready = matches!(&event, WorkerEvent::Ready(_));
        let bundle_reviewed = match &event {
            WorkerEvent::BundleImportReviewed(reviewed) => Some(*reviewed),
            _ => None,
        };
        let markdown_reviewed = match &event {
            WorkerEvent::MarkdownImportReviewed(reviewed) => Some(*reviewed),
            _ => None,
        };
        let markdown_review_discarded = match &event {
            WorkerEvent::MarkdownImportReviewDiscarded {
                request_id,
                review_request_id,
            } => Some((*request_id, *review_request_id)),
            _ => None,
        };
        let bundle_review_discarded = match &event {
            WorkerEvent::BundleImportReviewDiscarded {
                request_id,
                review_request_id,
            } => Some((*request_id, *review_request_id)),
            _ => None,
        };
        let imported_bundle = match &event {
            WorkerEvent::Accepted(accepted) => match accepted.result {
                ActionResult::ImportedBundle {
                    folder_count,
                    note_count,
                    attachment_count,
                    attachment_bytes,
                } => Some((
                    accepted.request_id,
                    BundleImportCompletion {
                        folder_count,
                        note_count,
                        attachment_count,
                        attachment_bytes,
                        maintenance_pending: accepted.commit.maintenance_pending,
                    },
                )),
                _ => None,
            },
            _ => None,
        };
        let attached_image = match &event {
            WorkerEvent::Accepted(accepted) => match accepted.result {
                ActionResult::AttachedImage {
                    note_id,
                    attachment_id,
                    ..
                } => Some((note_id, attachment_id)),
                _ => None,
            },
            _ => None,
        };
        let removed_attachment = match &event {
            WorkerEvent::Accepted(accepted) => match accepted.result {
                ActionResult::AttachmentReferenceRemoved { attachment_id, .. } => {
                    Some((accepted.request_id, attachment_id))
                }
                _ => None,
            },
            _ => None,
        };
        let chain_orphan = removed_attachment.is_some_and(|(request_id, attachment_id)| {
            self.attachment_remove_request == Some((request_id, attachment_id))
        });
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| self.attachment_request_id == Some(request_id))
            || matches!(&event, WorkerEvent::Ready(_)) && self.attachment_request_id.is_some()
        {
            self.attachment_request_id = None;
        }
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| {
                self.attachment_remove_request
                    .is_some_and(|(pending, _)| pending == request_id)
            })
            || matches!(&event, WorkerEvent::Ready(_)) && self.attachment_remove_request.is_some()
        {
            self.attachment_remove_request = None;
        }
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| {
                self.orphan_collection_request
                    .is_some_and(|(pending, _)| pending == request_id)
            })
            || matches!(&event, WorkerEvent::Ready(_)) && self.orphan_collection_request.is_some()
        {
            self.orphan_collection_request = None;
        }
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| self.note_import_request_id == Some(request_id))
        {
            self.note_import_request_id = None;
            self.markdown_import_review = None;
        }
        let tracked_export =
            exported.is_some_and(|exported| self.export_request_id == Some(exported.request_id));
        if tracked_export
            || rejected_request_id
                .is_some_and(|request_id| self.export_request_id == Some(request_id))
            || matches!(&event, WorkerEvent::Ready(_)) && self.export_request_id.is_some()
        {
            self.export_request_id = None;
        }
        let tracked_bundle_discard =
            bundle_review_discarded.is_some_and(|(request_id, review_request_id)| {
                self.bundle_action_request_id == Some(request_id)
                    && self.bundle_review_request_id == Some(review_request_id)
            });
        let tracked_bundle_import = imported_bundle
            .is_some_and(|(request_id, _)| self.bundle_action_request_id == Some(request_id));
        let tracked_markdown_discard =
            markdown_review_discarded.is_some_and(|(request_id, review_request_id)| {
                self.markdown_import_action_request_id == Some(request_id)
                    && self.note_import_request_id == Some(review_request_id)
            });
        let tracked_markdown_import = matches!(
            &event,
            WorkerEvent::Accepted(accepted)
                if self.markdown_import_action_request_id == Some(accepted.request_id)
                    && matches!(accepted.result, ActionResult::ImportedNote { .. })
        );
        if rejected_request_id
            .is_some_and(|request_id| self.markdown_import_action_request_id == Some(request_id))
        {
            self.markdown_import_action_request_id = None;
        }
        if rejected_request_id
            .is_some_and(|request_id| self.bundle_action_request_id == Some(request_id))
        {
            self.bundle_action_request_id = None;
        }
        if rejected_request_id.is_some_and(|request_id| {
            self.bundle_review_request_id == Some(request_id)
                && self.bundle_action_request_id != Some(request_id)
        }) {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
        }
        if matches!(&event, WorkerEvent::DraftReview(_)) {
            self.recovery_notice_dismissed = false;
        }
        let reveal_created = matches!(
            &event,
            WorkerEvent::Accepted(accepted)
                if matches!(
                    accepted.result,
                    ActionResult::CreatedNote(_) | ActionResult::ImportedNote { .. }
                )
        );
        let rejection = match &event {
            WorkerEvent::Rejected(rejected) => Some(worker_failure_message(rejected.failure)),
            WorkerEvent::Pending(pending) => Some(pending_message(pending.reason)),
            WorkerEvent::StartupFailed(error) => Some(error.to_string()),
            _ => None,
        };
        self.session.apply(event);
        after_session_apply(self);
        if let Some(reviewed) = markdown_reviewed
            .filter(|reviewed| self.note_import_request_id == Some(reviewed.request_id))
        {
            self.markdown_import_review = Some((
                reviewed.request_id,
                reviewed.base_library_revision,
                reviewed.review,
            ));
        }
        if tracked_markdown_discard || tracked_markdown_import {
            self.note_import_request_id = None;
            self.markdown_import_review = None;
            self.markdown_import_action_request_id = None;
        }
        if worker_ready
            && self.markdown_import_action_request_id.is_some()
            && self.note_import_request_id.is_some()
        {
            self.note_import_request_id = None;
            self.markdown_import_review = None;
            self.markdown_import_action_request_id = None;
        }
        if let Some(reviewed) = bundle_reviewed
            .filter(|reviewed| self.bundle_review_request_id == Some(reviewed.request_id))
        {
            self.bundle_review = Some((reviewed.request_id, reviewed.review));
        }
        if tracked_bundle_discard {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
            self.bundle_action_request_id = None;
        }
        if tracked_bundle_import {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
            self.bundle_action_request_id = None;
            self.bundle_import_completion = imported_bundle.map(|(_, completion)| completion);
        }
        if worker_ready
            && self.bundle_action_request_id.is_some()
            && self.bundle_review_request_id.is_some()
        {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
            self.bundle_action_request_id = None;
        }
        if let Some(ExportDialog::Review(review)) = self.export_dialog {
            if self
                .session
                .snapshot()
                .is_none_or(|snapshot| snapshot.revision != review.library_revision)
            {
                self.export_dialog = None;
                self.message = Some(
                    "The Notes library changed. Review the export again before choosing a destination."
                        .into(),
                );
            }
        }
        if let Some((note_id, attachment_id)) = attached_image {
            if self.session.selected_note_id() == Some(note_id) {
                self.selected_attachment = Some(attachment_id);
            }
        }
        let orphan_to_collect = chain_orphan.then(|| {
            removed_attachment
                .expect("a chained orphan comes from an accepted removal")
                .1
        });
        if let Some(accepted) = accepted_generation {
            if self
                .latest_local_generation
                .is_some_and(|latest| accepted >= latest)
            {
                self.latest_local_generation = None;
            }
        }
        if let Some(message) = rejection {
            self.message = Some(message.into());
        } else if sync_editor {
            self.message = None;
        }
        if sync_editor {
            self.sync_editor(window, cx);
        }
        if let Some(restored) = restored_draft {
            let decision = self
                .recovery_decision
                .take()
                .filter(|(note_id, _)| *note_id == restored.draft.note_id)
                .map(|(_, decision)| decision);
            match decision {
                Some(decision) => self.commit_restored_draft(restored, decision, window, cx),
                None => {
                    self.message = Some(
                        "Notes received an unexpected recovery response. The recovery record was preserved."
                            .into(),
                    );
                }
            }
        }
        if let Some(request_id) = rejected_request_id {
            if self
                .recovery_copy_pending
                .is_some_and(|(pending, _)| pending == request_id)
            {
                self.recovery_copy_pending = None;
            }
        }
        if let Some(request_id) = accepted_request_id {
            if let Some((_, draft_note_id)) = self
                .recovery_copy_pending
                .take_if(|(pending, _)| *pending == request_id)
            {
                self.discard_draft(draft_note_id, cx);
            }
        }
        if reveal_created {
            self.title.update(cx, |state, cx| state.focus(window, cx));
        }
        if let Some(attachment_id) = orphan_to_collect {
            self.queue_current_orphan_collection(attachment_id, cx);
        }
        if tracked_export {
            self.export_dialog = exported.map(|exported| ExportDialog::Complete(exported.outcome));
            self.message = None;
        }
        if refresh_search {
            self.dispatch_search(cx);
        }
        cx.notify();
    }
}
