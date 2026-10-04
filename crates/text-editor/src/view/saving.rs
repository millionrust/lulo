//! Text Editor revision-checked persistence and Save As orchestration.

use super::*;

struct NewPathRequest {
    content: SaveContent,
    format: document::TextFormat,
    then: Option<Pending>,
    forbidden_destination: Option<PathBuf>,
    suggested_name: Option<String>,
}

impl EditorView {
    /// The save sheet's disclosure triangle (macOS 26.2 dropped the Where
    /// pop-up's "Other…" item in favour of this control): open the full
    /// file-chooser browser for a custom destination.
    pub(super) fn open_full_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ActiveAlert::ConfirmSave(then)) = self.alert.take() {
            self.save_location = SaveLocation::Other;
            self.save_custom_folder = None;
            self.save_sheet(then, window, cx);
        }
    }

    pub(super) fn save_sheet(
        &mut self,
        then: Option<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.save_name_input.read(cx).text().to_string();
        let name = name.trim();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
        {
            self.alert = Some(ActiveAlert::ConfirmSave(then));
            self.status_notice = Some("Choose a valid document name.".into());
            cx.notify();
            return;
        }
        if self.save_location == SaveLocation::Other {
            self.save_to_new_path_request(
                NewPathRequest {
                    content: self.save_content(cx),
                    format: self.text_format,
                    then,
                    forbidden_destination: None,
                    suggested_name: Some(name.to_owned()),
                },
                window,
                cx,
            );
            return;
        }
        let Some(directory) = self
            .save_custom_folder
            .clone()
            .or_else(|| self.save_location.directory())
        else {
            self.alert = Some(ActiveAlert::ConfirmSave(then));
            return;
        };
        let content = self.save_content(cx);
        let path = directory.join(name);
        let path = if path.extension().is_none() {
            path.with_extension(content.default_extension())
        } else {
            path
        };
        let format = self.text_format;
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let save_content = content.clone();
            let saved_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if saved_path.symlink_metadata().is_ok() {
                        Err(SaveFailure::DestinationExists)
                    } else {
                        save_document_copy(&saved_path, None, &save_content, format)
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if result.is_ok() {
                    this.release_untitled_slot();
                    this.path = Some(path);
                }
                this.finish_document_save(result, content, then, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        if self.path.is_none() {
            self.show_save_sheet(None, window, cx);
        } else {
            self.save_with(None, window, cx);
        }
    }

    pub(super) fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        let content = self.save_content(cx);
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
        if self.file_busy {
            return;
        }
        if !self.dirty && self.path.is_some() {
            if let Some(pending) = then {
                self.perform(pending, window, cx);
            }
            return;
        }
        let content = self.save_content(cx);
        // A plain file made rich is never overwritten with RTF: it saves
        // under a new `.rtf` name, as TextEdit asks for one.
        let path = self.path.clone().filter(|_| !self.needs_rich_destination());
        if let Some(path) = path {
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
        content: SaveContent,
        format: document::TextFormat,
        then: Option<Pending>,
        forbidden_destination: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_to_new_path_request(
            NewPathRequest {
                content,
                format,
                then,
                forbidden_destination,
                suggested_name: None,
            },
            window,
            cx,
        );
    }

    fn save_to_new_path_request(
        &mut self,
        request: NewPathRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let NewPathRequest {
            content,
            format,
            then,
            forbidden_destination,
            suggested_name,
        } = request;
        let directory = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        // TextEdit's Save panel shows a never-saved document's name as plain
        // "Untitled" — no extension — because the extension is implied by
        // the format, not typed; a saved document's own name (with its
        // extension) is suggested as-is.
        let suggested_name = suggested_name.unwrap_or_else(|| {
            let current = self.path.as_deref();
            // A plain file made rich is offered under its `.rtf` name.
            let name = if content.is_rich() && current.is_some_and(|path| !is_rich_text_path(path))
            {
                current
                    .and_then(Path::file_stem)
                    .and_then(|stem| stem.to_str())
                    .map(|stem| format!("{stem}.rtf"))
            } else {
                current
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
            };
            name.unwrap_or_else(|| "Untitled".to_owned())
        });
        self.file_busy = true;
        cx.notify();
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
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
            // A name typed (or accepted) with no extension gets this plain
            // document's own, the same way the hidden extension in the
            // Save panel's Format popup would on the Mac.
            let path = if path.extension().is_none() {
                path.with_extension(content.default_extension())
            } else {
                path
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
                    this.release_untitled_slot();
                    this.path = Some(path);
                }
                this.finish_document_save(result, content, then, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn finish_document_save(
        &mut self,
        result: Result<SavedDocument, SaveFailure>,
        requested: SaveContent,
        then: Option<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_busy = false;
        match result {
            Ok(saved) => {
                match (saved, requested) {
                    (SavedDocument::Plain(saved), SaveContent::Plain(requested_text)) => {
                        if saved.text != requested_text {
                            self.install_document_text(saved.text, saved.longest_line, window, cx);
                        }
                        self.saved_bytes = Some(saved.original_bytes);
                        self.text_format = saved.format;
                    }
                    (SavedDocument::Plain(saved), SaveContent::Rich(_)) => {
                        self.saved_bytes = Some(saved.original_bytes);
                    }
                    (SavedDocument::Rich { original_bytes }, _) => {
                        self.saved_bytes = Some(original_bytes);
                    }
                }
                self.reset_document_watch();
                let recovery_cleared = self.mark_clean(cx);
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
