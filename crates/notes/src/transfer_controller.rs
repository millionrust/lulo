//! Notes attachment intake, import, and export orchestration.

use super::*;

impl NotesView {
    pub(super) fn export_note_markdown(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        let scope = ExportScope::Note {
            note_id: note.id,
            expected_note_revision: note.revision,
        };
        self.set_export_scope(scope, cx);
        self.choose_export_destination(ExportFormat::Markdown, cx);
    }

    pub(super) fn choose_image_attachment(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message = Some("Wait for this note to finish saving before adding a photo".into());
            cx.notify();
            return;
        }
        if self.session.selected_note().is_none_or(|note| note.deleted) {
            return;
        }
        self.attachment_chooser_open = true;
        self.message = None;
        cx.notify();
        let choice = crate::file_choosers::open(crate::file_choosers::OpenKind::Image, cx);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = choice.await;
            let _ = this.update(cx, |this, cx| {
                this.attachment_chooser_open = false;
                match choice {
                    Ok(Some(path)) => this.queue_image_attachment(path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message = Some("Notes could not open the image chooser".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn queue_image_attachment(
        &mut self,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            self.message = Some(
                "The note changed while the image chooser was open. Save it, then choose the image again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            self.message = Some("The selected note is no longer available".into());
            cx.notify();
            return;
        };
        let note_id = note.id;
        let expected_revision = note.revision;
        let modified_unix_ms = now_unix_ms()
            .max(note.created_unix_ms)
            .max(note.modified_unix_ms);
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::AttachImage {
                note_id,
                expected_revision,
                modified_unix_ms,
                selected_path,
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
            self.attachment_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn choose_text_note_import(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        self.note_import_chooser_open = true;
        self.message = None;
        cx.notify();
        let choice = crate::file_choosers::open(crate::file_choosers::OpenKind::TextNote, cx);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = choice.await;
            let _ = this.update(cx, |this, cx| {
                this.note_import_chooser_open = false;
                match choice {
                    Ok(Some(path)) => this.queue_text_note_import(path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message = Some("Notes could not open the note importer".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn queue_text_note_import(
        &mut self,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            self.message = Some(
                "The Notes library changed while the importer was open. Choose the file again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let folder_id = match self.session.folder_selection() {
            rmac_notes_runtime::FolderSelection::Folder(folder_id) => Some(folder_id),
            _ => None,
        };
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::ImportTextNote {
                created_unix_ms: now_unix_ms(),
                folder_id,
                selected_path,
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
            self.note_import_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn accept_markdown_import(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.markdown_import_action_request_id.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.note_import_request_id else {
            return;
        };
        let Some((reviewed_request_id, base_library_revision, _)) = self.markdown_import_review
        else {
            return;
        };
        if reviewed_request_id != review_request_id {
            return;
        }
        if self
            .session
            .snapshot()
            .is_none_or(|snapshot| snapshot.revision != base_library_revision)
        {
            self.message = Some(
                "The Notes library changed. Cancel this review and choose the Markdown file again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(
            WorkerCommand::AcceptMarkdownImport {
                request_id,
                review_request_id,
            },
            cx,
        ) {
            self.markdown_import_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn discard_markdown_import_review(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.markdown_import_action_request_id.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.note_import_request_id else {
            return;
        };
        if self
            .markdown_import_review
            .is_none_or(|(request_id, _, _)| request_id != review_request_id)
        {
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(
            WorkerCommand::DiscardMarkdownImportReview {
                request_id,
                review_request_id,
            },
            cx,
        ) {
            self.markdown_import_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn choose_bundle_import(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message =
                Some("Wait for this note to finish saving before importing a bundle".into());
            cx.notify();
            return;
        }
        self.bundle_chooser_open = true;
        self.message = None;
        cx.notify();
        let choice = crate::file_choosers::open(crate::file_choosers::OpenKind::Bundle, cx);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = choice.await;
            let _ = this.update(cx, |this, cx| {
                this.bundle_chooser_open = false;
                match choice {
                    Ok(Some(path)) => this.queue_bundle_import_review(path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message = Some("Notes could not open the bundle importer".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn queue_bundle_import_review(
        &mut self,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            self.message = Some(
                "The Notes library changed while the bundle chooser was open. Choose the bundle again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match BundleImportReviewRequest::new(request_id, selected_path) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::ReviewBundleImport(request), cx) {
            self.bundle_review_request_id = Some(request_id);
            self.bundle_review = None;
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn accept_bundle_import(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.bundle_action_request_id.is_some()
            || self.bundle_import_completion.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.bundle_review_request_id else {
            return;
        };
        let Some((reviewed_request_id, review)) = self.bundle_review else {
            return;
        };
        if reviewed_request_id != review_request_id {
            return;
        }
        if self
            .session
            .snapshot()
            .is_none_or(|snapshot| snapshot.revision != review.base_library_revision)
        {
            self.message = Some(
                "The Notes library changed. Cancel this review and choose the bundle again.".into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match BundleImportAcceptRequest::new(
            request_id,
            review_request_id,
            BundleCollisionPolicy::KeepBoth,
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::AcceptBundleImport(request), cx) {
            self.bundle_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn discard_bundle_import_review(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.bundle_action_request_id.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.bundle_review_request_id else {
            return;
        };
        if self
            .bundle_review
            .is_none_or(|(request_id, _)| request_id != review_request_id)
        {
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(
            WorkerCommand::DiscardBundleImportReview {
                request_id,
                review_request_id,
            },
            cx,
        ) {
            self.bundle_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn dismiss_bundle_import_completion(&mut self, cx: &mut Context<Self>) {
        self.bundle_import_completion = None;
        cx.notify();
    }

    pub(super) fn begin_export(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message = Some("Wait for this note to finish saving before exporting".into());
            cx.notify();
            return;
        }
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        let scope = if let Some(note) = self.session.selected_note() {
            ExportScope::Note {
                note_id: note.id,
                expected_note_revision: note.revision,
            }
        } else if let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        {
            let Some(folder) = snapshot
                .folders
                .iter()
                .find(|folder| folder.id == folder_id && !folder.deleted)
            else {
                return;
            };
            ExportScope::Folder {
                folder_id,
                expected_folder_revision: folder.revision,
            }
        } else {
            ExportScope::Library {
                expected_library_revision: snapshot.revision,
            }
        };
        self.set_export_scope(scope, cx);
    }

    pub(super) fn review_selected_note_export(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.session.selected_note() else {
            return;
        };
        self.set_export_scope(
            ExportScope::Note {
                note_id: note.id,
                expected_note_revision: note.revision,
            },
            cx,
        );
    }

    pub(super) fn review_current_folder_export(&mut self, cx: &mut Context<Self>) {
        let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        else {
            return;
        };
        let Some(folder) = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .folders
                .iter()
                .find(|folder| folder.id == folder_id && !folder.deleted)
        }) else {
            return;
        };
        self.set_export_scope(
            ExportScope::Folder {
                folder_id,
                expected_folder_revision: folder.revision,
            },
            cx,
        );
    }

    pub(super) fn review_library_export(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        self.set_export_scope(
            ExportScope::Library {
                expected_library_revision: snapshot.revision,
            },
            cx,
        );
    }

    pub(super) fn set_export_scope(&mut self, scope: ExportScope, cx: &mut Context<Self>) {
        let review = self
            .session
            .snapshot()
            .ok_or_else(|| "The Notes library is unavailable".to_string())
            .and_then(|snapshot| {
                snapshot
                    .plan_export(scope)
                    .map(|plan| ExportReview {
                        library_revision: plan.library_revision,
                        scope,
                        note_count: plan.note_ids.len(),
                        attachment_count: plan.attachments.len(),
                        markdown_bytes: plan.markdown_bytes,
                        attachment_bytes: plan.attachment_bytes,
                    })
                    .map_err(|error| error.to_string())
            });
        match review {
            Ok(review) => {
                self.export_dialog = Some(ExportDialog::Review(review));
                self.message = None;
            }
            Err(error) => {
                self.export_dialog = None;
                self.message = Some(error.into());
            }
        }
        cx.notify();
    }

    pub(super) fn choose_export_destination(
        &mut self,
        format: ExportFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(ExportDialog::Review(review)) = self.export_dialog else {
            return;
        };
        if format == ExportFormat::Markdown
            && (!matches!(review.scope, ExportScope::Note { .. }) || review.attachment_count != 0)
        {
            self.message =
                Some("Markdown export is available only for one note without attachments".into());
            cx.notify();
            return;
        }
        let suggested_name = self.export_suggested_name(review.scope, format);
        let kind = match format {
            ExportFormat::Markdown => crate::file_choosers::ExportKind::Markdown,
            ExportFormat::RmacBundle => crate::file_choosers::ExportKind::Bundle,
        };
        self.export_dialog = None;
        self.export_chooser_open = true;
        self.message = None;
        cx.notify();
        let destination = crate::file_choosers::export_destination(kind, &suggested_name, cx);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let destination = destination.await;
            let _ = this.update(cx, |this, cx| {
                this.export_chooser_open = false;
                match destination {
                    Ok(Some(path)) => this.queue_export(review.scope, format, path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message =
                            Some("Notes could not open the export destination chooser".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn queue_export(
        &mut self,
        scope: ExportScope,
        format: ExportFormat,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            self.message = Some("The Notes library changed. Review the export again.".into());
            cx.notify();
            return;
        }
        if let Err(error) = self
            .session
            .snapshot()
            .ok_or_else(|| "The Notes library is unavailable".to_string())
            .and_then(|snapshot| {
                snapshot
                    .plan_export(scope)
                    .map_err(|error| error.to_string())
            })
        {
            self.message = Some(error.into());
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ExportRequest::new(request_id, scope, format, selected_path) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Export(request), cx) {
            self.export_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn export_suggested_name(&self, scope: ExportScope, format: ExportFormat) -> String {
        let label = match scope {
            ExportScope::Note { note_id, .. } => self
                .session
                .snapshot()
                .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                .map_or_else(
                    || "Note".to_string(),
                    |note| display_title(&note.title).to_string(),
                ),
            ExportScope::Folder { folder_id, .. } => self
                .session
                .snapshot()
                .and_then(|snapshot| {
                    snapshot
                        .folders
                        .iter()
                        .find(|folder| folder.id == folder_id)
                })
                .map_or_else(|| "Notes Folder".to_string(), |folder| folder.name.clone()),
            ExportScope::Library { .. } => "All Notes".to_string(),
        };
        let extension = match format {
            ExportFormat::Markdown => "md",
            ExportFormat::RmacBundle => "rmacnotes",
        };
        format!("{}.{}", safe_export_stem(&label), extension)
    }

    pub(super) fn dismiss_export_dialog(&mut self, cx: &mut Context<Self>) {
        self.export_dialog = None;
        cx.notify();
    }

    /// Edit ▸ Attach File… (NOT-MENU-007). Notes' durable `AttachmentKind`
    /// is image-only (`rmac_notes_store::AttachmentKind`), so an arbitrary
    /// file is instead recorded as a `📎 filename` chip line inserted into
    /// the body — a real, durable line in the note's own Markdown source,
    /// not a stub. Reopening the file back up from that chip is a
    /// session-only convenience (`attachment_chip_paths`), since the chip
    /// is plain text with nothing else to identify the original path by
    /// after a relaunch (docs/parity.md).
    pub(super) fn choose_file_attachment(&mut self, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let choice = crate::file_choosers::attachments(cx);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = choice.await;
            let _ = this.update_in(cx, |this, window, cx| match choice {
                Ok(paths) => {
                    for path in paths {
                        this.insert_file_attachment_chip(path, window, cx);
                    }
                }
                Err(_) => {
                    this.message = Some("Notes could not open the file chooser".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn insert_file_attachment_chip(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        let note_id = note.id;
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let name = name.to_string();
        let chip = format!("📎 {name}");
        self.body.update(cx, |state, cx| {
            let value = state.value().to_string();
            let cursor = state.cursor();
            let needs_newline = cursor > 0 && !value[..cursor].ends_with('\n');
            let text = format!("{}{chip}\n", if needs_newline { "\n" } else { "" });
            state.set_selected_range(cursor..cursor, cx);
            state.replace(text, window, cx);
        });
        self.attachment_chip_paths
            .entry(note_id)
            .or_default()
            .insert(name, path);
        self.schedule_current_edit(cx);
        cx.notify();
    }

    /// A click on a `📎 filename` chip the current note's
    /// `attachment_chip_paths` still remembers: open the file with the
    /// desktop's default handler.
    pub(super) fn open_attachment_chip_path(path: std::path::PathBuf, cx: &mut Context<Self>) {
        crate::file_choosers::open_with_default_app(path, cx);
    }

    /// Edit ▸ Rename Attachment… (NOT-MENU-009): the selected attachment in
    /// the Attachments Browser (`selected_attachment`).
    pub(super) fn begin_rename_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(attachment_id) = self.selected_attachment else {
            return;
        };
        let Some(current_name) = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .attachments
                .iter()
                .find(|attachment| attachment.id == attachment_id && !attachment.deleted)
                .map(|attachment| attachment.display_name.clone())
        }) else {
            return;
        };
        let Some(revision) = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .attachments
                .iter()
                .find(|attachment| attachment.id == attachment_id)
                .map(|attachment| attachment.revision)
        }) else {
            return;
        };
        self.attachment_rename_input.update(cx, |input, cx| {
            input.set_value(current_name, window, cx);
        });
        self.attachment_rename = Some((attachment_id, revision));
        cx.notify();
    }

    pub(super) fn cancel_attachment_rename(&mut self, cx: &mut Context<Self>) {
        self.attachment_rename = None;
        cx.notify();
    }

    pub(super) fn commit_attachment_rename(&mut self, cx: &mut Context<Self>) {
        let Some((attachment_id, expected_revision)) = self.attachment_rename else {
            return;
        };
        let display_name = self.attachment_rename_input.read(cx).value().to_string();
        if display_name.trim().is_empty() {
            return;
        }
        self.send_action(
            LibraryAction::RenameAttachment {
                attachment_id,
                expected_attachment_revision: expected_revision,
                display_name,
            },
            cx,
        );
        self.attachment_rename = None;
        cx.notify();
    }
}
