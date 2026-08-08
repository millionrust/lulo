//! Text Editor external-change reload, copy, review, and overwrite authority.

use super::*;

impl EditorView {
    pub(super) fn show_external_conflict(&mut self, cx: &mut Context<Self>) {
        if !self.file_busy && !self.file_action_blocked() && self.external_change.is_some() {
            self.alert = Some(ActiveAlert::Conflict);
            cx.notify();
        }
    }

    pub(super) fn reload_conflicting_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.alert = None;
            return;
        };
        self.alert = None;
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { load_selected_document(&path) }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_busy = false;
                match loaded {
                    Ok(LoadedFile::Plain(document)) => {
                        this.input.update(cx, |state, cx| {
                            state.set_value(document.text.clone(), window, cx)
                        });
                        this.saved_bytes = Some(document.original_bytes);
                        this.text_format = document.format;
                        this.rtf_runs = None;
                        this.reset_document_watch();
                        this.mark_clean(document.text, cx);
                    }
                    Ok(LoadedFile::RichText { text, runs }) => {
                        this.input
                            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                        this.saved_bytes = None;
                        this.text_format = document::TextFormat::default();
                        this.rtf_runs = Some(runs);
                        this.reset_document_watch();
                        this.mark_clean(text, cx);
                    }
                    Err(message) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not reload the document.",
                            message,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_conflicting_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() {
            return;
        }
        self.alert = None;
        let content = self.input.read(cx).value().to_string();
        self.save_to_new_path(
            content,
            self.text_format,
            None,
            self.path.clone(),
            window,
            cx,
        );
    }

    pub(super) fn review_conflict_overwrite(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.alert = None;
            return;
        };
        self.alert = None;
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let reviewed = cx
                .background_executor()
                .spawn(async move {
                    storage::read_bounded(
                        &storage::RealStorage,
                        storage::Operation::ValidateDocumentRevision,
                        &path,
                        document::MAX_DOCUMENT_BYTES,
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.file_busy = false;
                this.alert = Some(match reviewed {
                    Ok(reviewed_revision) => ActiveAlert::ConfirmOverwrite { reviewed_revision },
                    Err(_) => ActiveAlert::Error {
                        title: "Could not review the external document.",
                        message: "The document is missing, inaccessible, or no longer within Text Editor’s safety limit. Your local buffer remains open; save a copy instead."
                            .into(),
                    },
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn overwrite_conflicting_document(
        &mut self,
        reviewed_revision: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy || self.rtf_runs.is_some() {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.alert = None;
            return;
        };
        self.alert = None;
        self.file_busy = true;
        let content = self.input.read(cx).value().to_string();
        let format = self.text_format;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let save_content = content.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    save_document(&path, Some(&reviewed_revision), &save_content, format)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_document_save(result, content, None, window, cx);
            });
        })
        .detach();
    }
}
