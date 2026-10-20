use super::*;

impl NotesView {
    pub(super) fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The same note again (its own save coming back): leave fields that
        // already hold the saved text alone, so the caret, selection and
        // undo history survive.
        let same_note = self.last_editor_note.is_some()
            && self.last_editor_note == self.session.selected_note_id();
        self.record_recent_note();
        if !same_note {
            // A different note's body makes any in-note Find state stale.
            self.note_find_open = false;
            self.note_find_matches.clear();
        }
        let (title, tags, body) = self
            .session
            .selected_note()
            .map(|note| (note.title.clone(), note.tags.clone(), note.body.clone()))
            .unwrap_or_default();
        let document = note_format_controller::body_document(&body);
        // The Tags field holds the tags the body's own `#tags` do not
        // (those stay in the body, where they were typed).
        let inline: std::collections::BTreeSet<String> = if self.smart_tags {
            tag_browser::inline_tags(&document.text())
                .into_iter()
                .map(|tag| tag.to_lowercase())
                .collect()
        } else {
            Default::default()
        };
        let tags = tags
            .into_iter()
            .filter(|tag| !inline.contains(&tag.to_lowercase()))
            .collect::<Vec<_>>()
            .join(", ");
        self.applying_snapshot = true;
        if !same_note || self.title.read(cx).value() != title {
            self.title
                .update(cx, |state, cx| state.set_value(title, window, cx));
        }
        let tags_shown = parse_tags(&self.tags.read(cx).value()).unwrap_or_default();
        if !same_note || tags_shown != parse_tags(&tags).unwrap_or_default() {
            self.tags
                .update(cx, |state, cx| state.set_value(tags, window, cx));
        }
        let shown = note_format_controller::body_markdown(self.body.read(cx).document());
        if !same_note || shown != body {
            self.body
                .update(cx, |editor, cx| editor.set_document(document, cx));
            self.maths_suggestion = None;
            self.schedule_spell_check(cx);
        }
        self.applying_snapshot = false;
        self.sync_attachment_preview(false, cx);
        self.sync_markdown_preview(cx);
    }

    /// Whether title/tags/body accept edits right now — mirrors
    /// `render_editor`'s own `editable` computation, so a `SetValue` action
    /// sent to a read-only editor (Recently Deleted, Markdown preview, no
    /// worker) is a no-op rather than silently changing on-screen text that
    /// then never saves.
    fn assistive_fields_editable(&self) -> bool {
        self.is_interactive_ready()
            && !self.markdown_preview_visible
            && !self.selected_note_closed()
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted)
    }

    /// AT-SPI's `SetValue`/`ReplaceSelectedText` for the title field, wired
    /// the same way `crates/launcher-app` wires Spotlight's search field:
    /// both actions replace the whole value, then run the same
    /// edit-scheduling path a keystroke takes.
    pub(super) fn assistive_title_listener(
        &self,
        cx: &Context<Self>,
    ) -> impl FnMut(Option<&accesskit::ActionData>, &mut Window, &mut gpui::App) + 'static {
        let view = cx.entity();
        move |data, window, cx| {
            let Some(accesskit::ActionData::Value(text)) = data else {
                return;
            };
            let text = text.to_string();
            view.update(cx, |this, cx| {
                if !this.assistive_fields_editable() {
                    return;
                }
                this.title
                    .update(cx, |state, cx| state.set_value(text, window, cx));
                this.schedule_current_edit(cx);
            });
        }
    }

    /// AT-SPI's `SetValue`/`ReplaceSelectedText` for the tags field. See
    /// [`Self::assistive_title_listener`].
    pub(super) fn assistive_tags_listener(
        &self,
        cx: &Context<Self>,
    ) -> impl FnMut(Option<&accesskit::ActionData>, &mut Window, &mut gpui::App) + 'static {
        let view = cx.entity();
        move |data, window, cx| {
            let Some(accesskit::ActionData::Value(text)) = data else {
                return;
            };
            let text = text.to_string();
            view.update(cx, |this, cx| {
                if !this.assistive_fields_editable() {
                    return;
                }
                this.tags
                    .update(cx, |state, cx| state.set_value(text, window, cx));
                this.schedule_current_edit(cx);
            });
        }
    }

    /// AT-SPI's `SetValue`/`ReplaceSelectedText` for the body field. See
    /// [`Self::assistive_title_listener`].
    pub(super) fn assistive_body_listener(
        &self,
        cx: &Context<Self>,
    ) -> impl FnMut(Option<&accesskit::ActionData>, &mut Window, &mut gpui::App) + 'static {
        let view = cx.entity();
        move |data, window, cx| {
            let Some(accesskit::ActionData::Value(text)) = data else {
                return;
            };
            let text = text.to_string();
            view.update(cx, |this, cx| {
                if !this.assistive_fields_editable() {
                    return;
                }
                this.body.update(cx, |editor, cx| {
                    let end = editor.document().len();
                    editor.replace_range(0..end, &text, cx);
                });
                let _ = window;
                this.schedule_current_edit(cx);
            });
        }
    }

    pub(super) fn schedule_current_edit(&mut self, cx: &mut Context<Self>) {
        // An edit while the print dialog is open makes its copy stale, and
        // the portal adapter then prints nothing and says why.
        self.print_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if self.applying_snapshot || !self.is_interactive_ready() {
            return;
        }
        let Some(note_id) = self.session.selected_note_id() else {
            return;
        };
        let title = self.title.read(cx).value().to_string();
        let body = note_format_controller::body_markdown(self.body.read(cx).document());
        let body_text = self.body_text(cx);
        let tag_text = self.tags.read(cx).value().to_string();
        if let Some(generation) =
            self.schedule_note_edit(note_id, title, &tag_text, body, &body_text, cx)
        {
            self.latest_local_generation = Some(generation);
        }
    }

    /// Schedule a save of note `note_id` with this title, Tags field text
    /// and body (stored form, and its plain text for `#tags`), from any
    /// window's editor. Returns the edit's generation once sent.
    pub(super) fn schedule_note_edit(
        &mut self,
        note_id: NoteId,
        title: String,
        tag_text: &str,
        body: String,
        body_text: &str,
        cx: &mut Context<Self>,
    ) -> Option<EditGeneration> {
        if !self.is_interactive_ready() {
            return None;
        }
        let note = self
            .session
            .snapshot()?
            .notes
            .iter()
            .find(|note| note.id == note_id)?;
        // A closed locked note shows no editor; never send its empty view.
        if note.deleted || self.session.is_note_closed(note) {
            return None;
        }
        self.last_activity = std::time::Instant::now();
        let expected_revision = note.revision;
        let created_unix_ms = note.created_unix_ms;
        let previous_modified = note.modified_unix_ms;
        let unchanged_title = title == note.title;
        let unchanged_body = body == note.body;
        let accepted_tags = note.tags.clone();
        let inline = if self.smart_tags {
            tag_browser::inline_tags(body_text)
        } else {
            Vec::new()
        };
        let tags = match parse_tags(tag_text) {
            Ok(tags) => tag_browser::merged_tags(tags, inline),
            Err(message) => {
                self.message = Some(message.into());
                cx.notify();
                return None;
            }
        };
        if unchanged_title && unchanged_body && tags == accepted_tags {
            return None;
        }
        let request_id = self.take_request_id()?;
        let generation = self.take_edit_generation()?;
        let modified_unix_ms = now_unix_ms().max(created_unix_ms).max(previous_modified);
        let edit = ScheduledEdit::new(
            request_id,
            generation,
            note_id,
            expected_revision,
            NoteChanges {
                modified_unix_ms,
                title,
                body,
                tags,
            },
        );
        match edit {
            Ok(edit) => self
                .send(WorkerCommand::ScheduleEdit(edit), cx)
                .then_some(generation),
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                None
            }
        }
    }

    pub(super) fn commit_restored_draft(
        &mut self,
        restored: rmac_notes_runtime::DraftRestoredEvent,
        decision: RecoveryDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let note_id = restored.draft.note_id;
        if decision == RecoveryDecision::PreserveCopy {
            self.preserve_recovered_copy(restored, cx);
            return;
        }
        let destination = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .notes
                .iter()
                .find(|note| note.id == note_id && !note.deleted)
                .map(|note| note.folder_id)
        });
        let Some(folder_id) = destination else {
            self.message = Some(
                "The recovered edit no longer has a safe destination. Its recovery record was preserved."
                    .into(),
            );
            cx.notify();
            return;
        };
        self.session.select_folder(folder_id.map_or(
            rmac_notes_runtime::FolderSelection::All,
            rmac_notes_runtime::FolderSelection::Folder,
        ));
        if !self.session.select_note(note_id) {
            self.message = Some(
                "The recovered edit could not be selected safely. Its recovery record was preserved."
                    .into(),
            );
            cx.notify();
            return;
        }

        let changes = restored.draft.changes.clone();
        self.applying_snapshot = true;
        self.title.update(cx, |state, cx| {
            state.set_value(changes.title.clone(), window, cx)
        });
        self.tags.update(cx, |state, cx| {
            state.set_value(changes.tags.join(", "), window, cx)
        });
        let document = note_format_controller::body_document(&changes.body);
        self.body
            .update(cx, |editor, cx| editor.set_document(document, cx));
        self.applying_snapshot = false;

        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let Some(generation) = self.take_edit_generation() else {
            return;
        };
        match ScheduledEdit::new(
            request_id,
            generation,
            note_id,
            restored.draft.base_note_revision,
            changes,
        ) {
            Ok(edit) => {
                self.message = Some("Restoring the recovered edit…".into());
                if self.send(WorkerCommand::ScheduleEdit(edit), cx) {
                    self.latest_local_generation = Some(generation);
                }
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn preserve_recovered_copy(
        &mut self,
        restored: rmac_notes_runtime::DraftRestoredEvent,
        cx: &mut Context<Self>,
    ) {
        let draft_note_id = restored.draft.note_id;
        let folder_id = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .notes
                .iter()
                .find(|note| note.id == draft_note_id && !note.deleted)
                .and_then(|note| note.folder_id)
        });
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let changes = restored.draft.changes;
        let action = LibraryAction::CreateNote(NewNote {
            created_unix_ms: now_unix_ms(),
            title: changes.title,
            body: changes.body,
            tags: changes.tags,
            folder_id,
        });
        let request = match ActionRequest::new(request_id, action) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.recovery_copy_pending = Some((request_id, draft_note_id));
            self.message = Some("Preserving the recovered edit as a new note…".into());
            cx.notify();
        }
    }

    pub(super) fn discard_pending(&mut self, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(WorkerCommand::DiscardPending { request_id }, cx) {
            self.latest_local_generation = None;
        }
    }

    pub(super) fn restore_draft(
        &mut self,
        note_id: NoteId,
        decision: RecoveryDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.recovery_decision = Some((note_id, decision));
        if !self.send(
            WorkerCommand::RestoreDraft {
                request_id,
                note_id,
            },
            cx,
        ) {
            self.recovery_decision = None;
        }
    }

    pub(super) fn discard_draft(&mut self, note_id: NoteId, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(
            WorkerCommand::DiscardDraft {
                request_id,
                note_id,
            },
            cx,
        );
    }

    pub(super) fn continue_after_recovery_notice(&mut self, cx: &mut Context<Self>) {
        if self
            .session
            .draft_review()
            .is_some_and(|review| review.drafts.is_empty())
        {
            self.recovery_notice_dismissed = true;
            self.message = None;
            cx.notify();
        }
    }
}
