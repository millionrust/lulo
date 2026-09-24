use super::*;

impl NotesView {
    pub(super) fn is_interactive_ready(&self) -> bool {
        matches!(self.session.phase(), SessionPhase::Ready)
            && !self.recovery_review_is_blocking()
            && self.folder_dialog.is_none()
            && self.purge_dialog.is_none()
            && self.move_dialog.is_none()
            && self.attachment_dialog.is_none()
            && self.export_dialog.is_none()
            && !self.attachment_chooser_open
            && !self.note_import_chooser_open
            && self.note_import_request_id.is_none()
            && self.markdown_import_review.is_none()
            && self.markdown_import_action_request_id.is_none()
            && !self.export_chooser_open
            && self.export_request_id.is_none()
            && !self.bundle_chooser_open
            && self.bundle_review_request_id.is_none()
            && self.bundle_review.is_none()
            && self.bundle_action_request_id.is_none()
            && self.bundle_import_completion.is_none()
            && !self.attachment_action_pending()
    }

    pub(super) fn attachment_action_pending(&self) -> bool {
        self.attachment_request_id.is_some()
            || self.attachment_remove_request.is_some()
            || self.orphan_collection_request.is_some()
    }

    pub(super) fn recovery_review_is_blocking(&self) -> bool {
        self.session.draft_review().is_some() && !self.recovery_notice_dismissed
    }

    pub(super) fn send_action(&mut self, action: LibraryAction, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        match ActionRequest::new(request_id, action) {
            Ok(request) => {
                self.send(WorkerCommand::Apply(request), cx);
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    pub(super) fn send(&mut self, command: WorkerCommand, cx: &mut Context<Self>) -> bool {
        let result = self
            .worker
            .as_ref()
            .ok_or(WorkerSendError::Closed)
            .and_then(|worker| worker.try_send(command));
        if let Err(error) = result {
            self.message = Some(error.to_string().into());
            cx.notify();
            false
        } else {
            true
        }
    }

    pub(super) fn take_request_id(&mut self) -> Option<u64> {
        take_counter(&mut self.next_request_id).or_else(|| {
            self.message = Some("Notes exhausted its request identity sequence".into());
            None
        })
    }

    pub(super) fn take_edit_generation(&mut self) -> Option<EditGeneration> {
        take_counter(&mut self.next_edit_generation)
            .and_then(EditGeneration::new)
            .or_else(|| {
                self.message = Some("Notes exhausted its edit generation sequence".into());
                None
            })
    }

    pub(super) fn continue_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy {
            // `print_busy` also covers Export as PDF, the same print pipeline
            // without a portal dialog of its own.
            self.message = Some("Finish or cancel the print or export before closing Notes".into());
            cx.notify();
            return;
        }
        if self.attachment_chooser_open {
            self.message = Some("Finish or cancel the image chooser before closing Notes".into());
            cx.notify();
            return;
        }
        if self.note_import_chooser_open {
            self.message = Some("Finish or cancel the note importer before closing Notes".into());
            cx.notify();
            return;
        }
        if self.markdown_import_action_request_id.is_some() {
            self.message = Some("Wait for the current Markdown import action to finish".into());
            cx.notify();
            return;
        }
        if self.note_import_request_id.is_some() {
            self.message = Some(if self.markdown_import_review.is_some() {
                "Import or cancel the reviewed Markdown file before closing Notes.".into()
            } else {
                "Wait for the selected note file to finish its private review.".into()
            });
            cx.notify();
            return;
        }
        if self.export_chooser_open {
            self.message = Some("Finish or cancel the export chooser before closing Notes".into());
            cx.notify();
            return;
        }
        if self.export_request_id.is_some() {
            self.message = Some("Wait for the Notes export to finish".into());
            cx.notify();
            return;
        }
        if self.bundle_chooser_open {
            self.message = Some("Finish or cancel the bundle chooser before closing Notes".into());
            cx.notify();
            return;
        }
        if self.bundle_action_request_id.is_some() {
            self.message = Some("Wait for the current bundle operation to finish".into());
            cx.notify();
            return;
        }
        if self.bundle_review_request_id.is_some() {
            self.message = Some(if self.bundle_review.is_some() {
                "Import or cancel the reviewed bundle before closing Notes so its private review can be released."
                    .into()
            } else {
                "Wait for the selected Notes bundle to finish its private review before closing."
                    .into()
            });
            cx.notify();
            return;
        }
        if self.attachment_action_pending() {
            self.message = Some("Wait for the current attachment operation to finish".into());
            cx.notify();
            return;
        }
        if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
            self.message = Some("Retry or discard the pending change before closing Notes".into());
            cx.notify();
            return;
        }
        if !self.request_markdown_preview_shutdown(cx) {
            return;
        }
        if !self.request_preview_shutdown(cx) {
            return;
        }
        if !self.request_search_shutdown(cx) {
            return;
        }
        let shutdown = self
            .worker
            .as_ref()
            .ok_or(WorkerSendError::Closed)
            .and_then(|worker| worker.try_send(WorkerCommand::Shutdown));
        if let Err(error) = shutdown {
            self.message =
                Some(format!("Notes could not safely close yet: {error}. Try again.").into());
            cx.notify();
            return;
        }
        self.closing = true;
        window.remove_window();
    }
}

impl Drop for NotesView {
    fn drop(&mut self) {
        if !self.closing {
            self.markdown_preview.clear();
            if let Some(markdown_preview_worker) = &self.markdown_preview_worker {
                if matches!(
                    markdown_preview_worker.try_shutdown(),
                    Err(MarkdownPreviewWorkerSendError::Full)
                ) {
                    let markdown_preview_worker = markdown_preview_worker.clone();
                    let _ = thread::Builder::new()
                        .name("rmac-notes-markdown-preview-close".into())
                        .spawn(move || {
                            let _ = markdown_preview_worker.shutdown_blocking();
                        });
                }
            }
            self.preview.clear();
            if let Some(preview_worker) = &self.preview_worker {
                if matches!(
                    preview_worker.try_shutdown(),
                    Err(PreviewWorkerSendError::Full)
                ) {
                    let preview_worker = preview_worker.clone();
                    let _ = thread::Builder::new()
                        .name("rmac-notes-preview-close".into())
                        .spawn(move || {
                            let _ = preview_worker.shutdown_blocking();
                        });
                }
            }
            self.search.cancel();
            if let Some(search_worker) = &self.search_worker {
                if matches!(
                    search_worker.try_shutdown(),
                    Err(SearchWorkerSendError::Full)
                ) {
                    let search_worker = search_worker.clone();
                    let _ = thread::Builder::new()
                        .name("rmac-notes-search-close".into())
                        .spawn(move || {
                            let _ = search_worker.shutdown_blocking();
                        });
                }
            }
            if let Some(worker) = &self.worker {
                let _ = worker.try_send(WorkerCommand::Shutdown);
            }
        }
    }
}
