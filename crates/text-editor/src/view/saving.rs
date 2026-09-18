//! Text Editor revision-checked persistence and Save As orchestration.

use super::*;

impl EditorView {
    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        self.save_with(None, window, cx);
    }

    pub(super) fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked() {
            return;
        }
        let content = self.input.read(cx).value().to_string();
        self.save_to_new_path(content, self.text_format, None, None, window, cx);
    }

    /// Save the buffer; if `then` is set, run that pending action only **after**
    /// the save has actually succeeded (important for the async Save-As path so
    /// the destructive action never runs before the file is written).
    pub(super) fn save_with(
        &mut self,
        then: Option<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The RTF preview is read-only — never write plain text over the .rtf.
        if self.rtf_runs.is_some() {
            return;
        }
        if self.file_busy {
            return;
        }
        if !self.dirty && self.path.is_some() {
            if let Some(pending) = then {
                self.perform(pending, window, cx);
            }
            return;
        }
        let content = self.input.read(cx).value().to_string();
        if let Some(path) = self.path.clone() {
            let Some(expected) = self.saved_bytes.clone() else {
                self.alert = Some(ActiveAlert::Error {
                    title: "The file could not be saved.",
                    message: "Text Editor could not validate the opened document revision. Save a copy instead."
                        .into(),
                });
                cx.notify();
                return;
            };
            let format = self.text_format;
            self.file_busy = true;
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let save_content = content.clone();
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { save_document(&path, Some(&expected), &save_content, format) },
                    )
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finish_document_save(result, content, then, window, cx);
                });
            })
            .detach();
            return;
        }
        self.save_to_new_path(content, self.text_format, then, None, window, cx);
    }

    pub(super) fn save_to_new_path(
        &mut self,
        content: String,
        format: document::TextFormat,
        then: Option<Pending>,
        forbidden_destination: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested_name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled.txt");
        self.file_busy = true;
        cx.notify();
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested_name));
        cx.spawn_in(window, async move |this, cx| {
            // Save-As was cancelled or failed: do NOT run the pending action,
            // so unsaved changes are preserved instead of silently discarded.
            let picker = receiver.await;
            let Ok(Ok(Some(path))) = picker else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.file_busy = false;
                    if !matches!(picker, Ok(Ok(None))) {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not open the save dialog.",
                            message: "The desktop file chooser is temporarily unavailable.".into(),
                        });
                    }
                    cx.notify();
                });
                return;
            };
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    let content = content.clone();
                    async move {
                        save_document_copy(
                            &path,
                            forbidden_destination.as_deref(),
                            &content,
                            format,
                        )
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if result.is_ok() {
                    this.path = Some(path);
                }
                this.finish_document_save(result, content, then, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn finish_document_save(
        &mut self,
        result: Result<document::DecodedDocument, SaveFailure>,
        requested_text: String,
        then: Option<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_busy = false;
        match result {
            Ok(saved) => {
                if saved.text != requested_text {
                    self.input.update(cx, |state, cx| {
                        state.set_value(saved.text.clone(), window, cx)
                    });
                }
                self.saved_bytes = Some(saved.original_bytes);
                self.text_format = saved.format;
                self.reset_document_watch();
                let recovery_cleared = self.mark_clean(saved.text, cx);
                self.record_current_document(cx);
                if recovery_cleared {
                    if let Some(pending) = then {
                        self.perform(pending, window, cx);
                    }
                }
            }
            Err(error) => {
                if matches!(
                    &error,
                    SaveFailure::Storage(storage::SaveDocumentError::Conflict)
                ) {
                    self.external_change = Some(ExternalChange::Modified);
                }
                self.alert = Some(
                    if matches!(
                        &error,
                        SaveFailure::Storage(storage::SaveDocumentError::Conflict)
                    ) {
                        ActiveAlert::Conflict
                    } else {
                        ActiveAlert::Error {
                            title: "The file could not be saved.",
                            message: error.to_string(),
                        }
                    },
                );
            }
        }
        cx.notify();
    }
}
