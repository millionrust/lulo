//! Text Editor new-window, portal selection, RTF conversion, and document loading.

use super::*;

impl EditorView {
    pub(super) fn open_pending_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_open_picker && !self.file_busy && !self.file_action_blocked() {
            self.pending_open_picker = false;
            self.open(window, cx);
        }
    }

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

    /// File ▸ Open Recent ▸ (TE-02): opens the document at `index` in the
    /// store's own File-Open-Recent list, re-read now (off the render
    /// thread) rather than cached from when the menu opened. Reuses this
    /// window when it is a clean empty untitled one, exactly as Open… does;
    /// a document the store no longer lists (moved, deleted, or the list
    /// simply changed since the menu opened) is silently skipped.
    pub(super) fn open_recent(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let path = cx
                .background_executor()
                .spawn(async move {
                    let store = rmac_recent_documents::Store::from_environment().ok()?;
                    let mut paths = store.load_for_app(rmac_ui::app_id::TEXT_EDITOR).ok()?;
                    (index < paths.len()).then(|| paths.swap_remove(index))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_busy = false;
                if let Some(path) = path {
                    let reuse_current = should_reuse_untitled_window(
                        this.dirty,
                        this.path.is_some(),
                        this.body_is_empty(cx),
                    );
                    if reuse_current {
                        this.load_document_path(path, "The file could not be opened.", window, cx);
                    } else if open_editor_window(cx, Some(path)).is_err() {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not open a new document window.",
                            message: "Text Editor could not create another window. This document \
                                      remains open."
                                .into(),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
            rich: self
                .rich_text
                .then(|| self.rich.read(cx).document().clone()),
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

    fn do_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        self.file_busy = true;
        let reuse_current =
            should_reuse_untitled_window(self.dirty, self.path.is_some(), self.body_is_empty(cx));
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
                        this.prevent_editing = false;
                        this.rich_text = false;
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
                        this.reset_document_watch();
                        this.mark_clean(cx);
                        this.record_current_document(cx);
                    }
                    Ok(LoadedFile::RichText {
                        document,
                        original_bytes,
                    }) => {
                        this.prevent_editing = false;
                        this.input
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        this.install_rich_document(document, cx);
                        this.release_untitled_slot();
                        this.path = Some(path);
                        this.saved_bytes = Some(original_bytes);
                        this.text_format = document::TextFormat::default();
                        this.reset_document_watch();
                        this.mark_clean(cx);
                        this.record_current_document(cx);
                        this.focus_body(window, cx);
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
