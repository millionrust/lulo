//! Text Editor new-window, portal selection, RTF conversion, and document loading.

use super::*;

impl EditorView {
    pub(super) fn new_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        if open_editor_window(cx, None).is_err() {
            self.alert = Some(ActiveAlert::Error {
                title: "Could not open a new document window.",
                message: "Text Editor could not create another window. This document remains open."
                    .into(),
            });
            cx.notify();
        }
    }

    pub(super) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        self.do_open(window, cx);
    }

    /// TextEdit's File ▸ Duplicate: a new window with this document's exact
    /// content, unsaved. Unlike Save As, the window this was invoked from
    /// keeps its own path and dirty state untouched.
    pub(super) fn duplicate_document(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        let content = super::DuplicateContent {
            text: self.document_text(cx),
            format: self.text_format,
            mono: self.mono,
            font_size: self.font_size,
            rtf_runs: self.rtf_runs.clone(),
        };
        if open_duplicate_window(cx, content).is_err() {
            self.alert = Some(ActiveAlert::Error {
                title: "Could not open a duplicate window.",
                message: "Text Editor could not create another window. This document remains open."
                    .into(),
            });
            cx.notify();
        }
    }

    /// Leave the read-only RTF preview and continue editing the extracted
    /// text as a new untitled plain-text document — the original `.rtf` is
    /// never overwritten. TextEdit always warns before a rich document loses
    /// its formatting this way, so this only asks; [`Self::alert_confirm`]
    /// calls [`Self::perform_edit_as_plain_text`] once the user agrees.
    pub(super) fn edit_as_plain_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.file_action_blocked() || self.rtf_runs.is_none() {
            return;
        }
        self.alert = Some(ActiveAlert::ConfirmPlainTextConversion);
        cx.notify();
    }

    /// The confirmed conversion itself — see [`Self::edit_as_plain_text`].
    pub(super) fn perform_edit_as_plain_text(&mut self, cx: &mut Context<Self>) {
        if self.rtf_runs.take().is_some() {
            self.path = None;
            self.saved_bytes = None;
            self.text_format = document::TextFormat::default();
            self.reset_document_watch();
            self.dirty = true;
            self.report_unsaved(cx);
            self.schedule_autosave(cx);
            cx.notify();
        }
    }

    fn do_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        self.file_busy = true;
        let reuse_current = should_reuse_untitled_window(
            self.dirty,
            self.path.is_some(),
            self.rtf_runs.is_some(),
            self.input.read(cx).text().len() == 0 && self.long_lines.is_none(),
        );
        cx.notify();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let picker = receiver.await;
            let Ok(Ok(Some(paths))) = picker else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.file_busy = false;
                    if !matches!(picker, Ok(Ok(None))) {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not open the file chooser.",
                            message: "The desktop file chooser is temporarily unavailable.".into(),
                        });
                    }
                    cx.notify();
                });
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_busy = false;
                let mut paths = paths.into_iter();
                if reuse_current {
                    if let Some(path) = paths.next() {
                        this.load_document_path(path, "The file could not be opened.", window, cx);
                    }
                }
                let mut failed_windows = 0_usize;
                for path in paths {
                    if open_editor_window(cx, Some(path)).is_err() {
                        failed_windows += 1;
                    }
                }
                if failed_windows > 0 {
                    this.status_notice = Some(
                        format!(
                            "Text Editor could not create {} selected document {}.",
                            failed_windows,
                            if failed_windows == 1 {
                                "window"
                            } else {
                                "windows"
                            }
                        )
                        .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn load_document_path(
        &mut self,
        path: PathBuf,
        error_title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy {
            return;
        }
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
                        this.install_document_text(
                            document.text,
                            document.longest_line,
                            window,
                            cx,
                        );
                        this.release_untitled_slot();
                        this.path = Some(path);
                        this.saved_bytes = Some(document.original_bytes);
                        this.text_format = document.format;
                        this.rtf_runs = None;
                        this.reset_document_watch();
                        this.mark_clean(cx);
                        this.record_current_document(cx);
                    }
                    Ok(LoadedFile::RichText { text, runs }) => {
                        this.long_lines = None;
                        this.input
                            .update(cx, |state, cx| state.set_value(text, window, cx));
                        this.text_revision = this.text_revision.wrapping_add(1);
                        this.release_untitled_slot();
                        this.path = Some(path);
                        this.saved_bytes = None;
                        this.text_format = document::TextFormat::default();
                        this.rtf_runs = Some(runs);
                        this.reset_document_watch();
                        this.mark_clean(cx);
                        this.record_current_document(cx);
                    }
                    Err(message) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: error_title,
                            message,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
