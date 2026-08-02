//! Notes Markdown/image preview and attachment-cleanup orchestration.

use super::*;

impl NotesView {
    pub(super) fn sync_markdown_preview(&mut self, cx: &mut Context<Self>) {
        if !self.markdown_preview_visible {
            self.markdown_preview.clear();
            return;
        }
        let Some(snapshot) = self.session.snapshot() else {
            self.markdown_preview.clear();
            return;
        };
        let Some(note) = self.session.selected_note() else {
            self.markdown_preview.clear();
            return;
        };
        let Some(worker) = self.markdown_preview_worker.clone() else {
            self.markdown_preview.clear();
            self.message = Some("Notes Markdown preview is unavailable".into());
            cx.notify();
            return;
        };
        let request = match self.markdown_preview.request(
            snapshot.revision,
            note.id,
            note.revision,
            Arc::<str>::from(note.body.as_str()),
        ) {
            Ok(request) => request,
            Err(error) => {
                self.markdown_preview.clear();
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if let Err(error) = worker.try_run(request) {
            self.markdown_preview.clear();
            self.message = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(super) fn toggle_markdown_preview(&mut self, cx: &mut Context<Self>) {
        if self.markdown_preview_visible {
            self.markdown_preview_visible = false;
            self.markdown_preview.clear();
            cx.notify();
            return;
        }
        if !self.is_interactive_ready() || self.session.selected_note().is_none() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message = Some("Wait for this note to finish saving before previewing it".into());
            cx.notify();
            return;
        }
        self.markdown_preview_visible = true;
        self.message = None;
        self.sync_markdown_preview(cx);
    }

    pub(super) fn retry_markdown_preview(&mut self, cx: &mut Context<Self>) {
        if self.markdown_preview_visible {
            self.sync_markdown_preview(cx);
        }
    }

    pub(super) fn sync_attachment_preview(&mut self, force: bool, cx: &mut Context<Self>) {
        let candidate = self.session.snapshot().and_then(|snapshot| {
            let note = self.session.selected_note()?;
            let selected = self
                .selected_attachment
                .filter(|id| note.attachments.contains(id))
                .or_else(|| note.attachments.first().copied());
            let attachment_id = selected?;
            let attachment = snapshot
                .attachments
                .iter()
                .find(|attachment| attachment.id == attachment_id && !attachment.deleted)?
                .clone();
            Some((snapshot.revision, attachment_id, attachment))
        });
        let Some((library_revision, attachment_id, attachment)) = candidate else {
            self.selected_attachment = None;
            self.preview.clear();
            self.preview_image = None;
            cx.notify();
            return;
        };
        self.selected_attachment = Some(attachment_id);
        let current_is_exact = match self.preview.state() {
            PreviewState::Loading {
                library_revision: current_revision,
                attachment_id: current_attachment,
                ..
            }
            | PreviewState::Unavailable {
                library_revision: current_revision,
                attachment_id: current_attachment,
                ..
            } => *current_revision == library_revision && *current_attachment == attachment_id,
            PreviewState::Ready {
                library_revision: current_revision,
                image,
                ..
            } => *current_revision == library_revision && image.attachment_id() == attachment_id,
            PreviewState::Empty => false,
        };
        if current_is_exact && !force {
            return;
        }
        let Some(worker) = self.preview_worker.clone() else {
            self.preview.clear();
            self.preview_image = None;
            cx.notify();
            return;
        };
        let target = match PreviewSize::new(720, 360) {
            Ok(target) => target,
            Err(error) => {
                self.preview.clear();
                self.preview_image = None;
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let request = match self.preview.request(library_revision, attachment, target) {
            Ok(request) => request,
            Err(error) => {
                self.preview.clear();
                self.preview_image = None;
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.preview_image = None;
        if let Err(error) = worker.try_run(request) {
            self.preview.clear();
            self.message = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub(super) fn select_attachment_preview(
        &mut self,
        attachment_id: AttachmentId,
        cx: &mut Context<Self>,
    ) {
        let belongs_to_note = self
            .session
            .selected_note()
            .is_some_and(|note| note.attachments.contains(&attachment_id));
        if !belongs_to_note {
            return;
        }
        self.selected_attachment = Some(attachment_id);
        self.sync_attachment_preview(true, cx);
    }

    pub(super) fn retry_attachment_preview(&mut self, cx: &mut Context<Self>) {
        self.sync_attachment_preview(true, cx);
    }

    pub(super) fn first_orphaned_attachment(&self) -> Option<&rmac_notes_store::AttachmentRecord> {
        self.session
            .snapshot()?
            .attachments
            .iter()
            .find(|attachment| attachment.deleted)
    }

    pub(super) fn begin_attachment_removal(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message =
                Some("Wait for this note to finish saving before removing a photo".into());
            cx.notify();
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        let Some(attachment_id) = self.selected_attachment else {
            return;
        };
        if !note.attachments.contains(&attachment_id) {
            return;
        }
        let Some(attachment) = self.session.snapshot().and_then(|snapshot| {
            snapshot.attachments.iter().find(|attachment| {
                attachment.id == attachment_id
                    && attachment.note_id == note.id
                    && !attachment.deleted
            })
        }) else {
            return;
        };
        self.attachment_dialog = Some(AttachmentDialog::Remove {
            note_id: note.id,
            note_revision: note.revision,
            attachment_id,
            attachment_revision: attachment.revision,
            byte_len: attachment.byte_len,
        });
        cx.notify();
    }

    pub(super) fn begin_orphan_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(attachment) = self.first_orphaned_attachment() else {
            return;
        };
        self.attachment_dialog = Some(AttachmentDialog::CollectOrphan {
            attachment_id: attachment.id,
            attachment_revision: attachment.revision,
            byte_len: attachment.byte_len,
        });
        cx.notify();
    }

    pub(super) fn confirm_attachment_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.attachment_dialog.take() else {
            return;
        };
        match dialog {
            AttachmentDialog::Remove {
                note_id,
                note_revision,
                attachment_id,
                attachment_revision,
                ..
            } => self.queue_attachment_removal(
                note_id,
                note_revision,
                attachment_id,
                attachment_revision,
                cx,
            ),
            AttachmentDialog::CollectOrphan {
                attachment_id,
                attachment_revision,
                ..
            } => self.queue_orphan_collection(attachment_id, attachment_revision, cx),
        }
    }

    pub(super) fn cancel_attachment_dialog(&mut self, cx: &mut Context<Self>) {
        self.attachment_dialog = None;
        cx.notify();
    }

    pub(super) fn queue_attachment_removal(
        &mut self,
        note_id: NoteId,
        expected_note_revision: u64,
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            self.message = Some(
                "The note changed before the image could be removed. Review the removal again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(note) = self
            .session
            .selected_note()
            .filter(|note| note.id == note_id && !note.deleted)
        else {
            self.message = Some("The selected note is no longer available".into());
            cx.notify();
            return;
        };
        let modified_unix_ms = now_unix_ms()
            .max(note.created_unix_ms)
            .max(note.modified_unix_ms);
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::RemoveAttachmentReference {
                note_id,
                expected_note_revision,
                attachment_id,
                expected_attachment_revision,
                modified_unix_ms,
            },
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.attachment_remove_request = Some((request_id, attachment_id));
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn queue_current_orphan_collection(
        &mut self,
        attachment_id: AttachmentId,
        cx: &mut Context<Self>,
    ) {
        let Some(attachment_revision) = self
            .session
            .snapshot()
            .and_then(|snapshot| {
                snapshot
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == attachment_id && attachment.deleted)
            })
            .map(|attachment| attachment.revision)
        else {
            cx.notify();
            return;
        };
        self.queue_orphan_collection(attachment_id, attachment_revision, cx);
    }

    pub(super) fn queue_orphan_collection(
        &mut self,
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.session.phase(), SessionPhase::Ready) || self.attachment_action_pending()
        {
            cx.notify();
            return;
        }
        let exact_orphan_exists = self.session.snapshot().is_some_and(|snapshot| {
            snapshot.attachments.iter().any(|attachment| {
                attachment.id == attachment_id
                    && attachment.deleted
                    && attachment.revision == expected_attachment_revision
            })
        });
        if !exact_orphan_exists {
            self.message = Some(
                "The removed attachment changed before cleanup. Review the cleanup again.".into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::CollectOrphanedAttachment {
                attachment_id,
                expected_attachment_revision,
            },
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.orphan_collection_request = Some((request_id, attachment_id));
            self.message = None;
            cx.notify();
        }
    }
}
