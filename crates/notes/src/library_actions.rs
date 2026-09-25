use super::*;

impl NotesView {
    pub(super) fn select_folder(
        &mut self,
        folder: rmac_notes_runtime::FolderSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        let previous = self.session.selected_note_id();
        self.session.select_folder(folder);
        if self.session.selected_note_id() != previous || self.latest_local_generation.is_none() {
            self.sync_editor(window, cx);
        }
        cx.notify();
    }

    pub(super) fn select_note(
        &mut self,
        note_id: NoteId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        let previous = self.session.selected_note_id();
        if self.session.select_note(note_id) {
            if previous != Some(note_id) || self.latest_local_generation.is_none() {
                self.sync_editor(window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn create_note(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let folder_id = match self.session.folder_selection() {
            rmac_notes_runtime::FolderSelection::Folder(folder_id) => Some(folder_id),
            _ => None,
        };
        self.send_action(
            LibraryAction::CreateNote(NewNote {
                created_unix_ms: now_unix_ms(),
                title: "New Note".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id,
            }),
            cx,
        );
    }

    pub(super) fn create_folder(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let existing = self
            .session
            .folders()
            .into_iter()
            .map(|folder| folder.name.to_lowercase())
            .collect::<Vec<_>>();
        let name = unique_folder_name(&existing);
        self.send_action(LibraryAction::CreateFolder { name }, cx);
    }

    pub(super) fn begin_folder_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        else {
            return;
        };
        let Some(name) = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.name.clone())
        else {
            return;
        };
        self.folder_name_input
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.folder_dialog = Some(FolderDialog::Rename(folder_id));
        self.folder_name_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn commit_folder_rename(&mut self, cx: &mut Context<Self>) {
        let Some(FolderDialog::Rename(folder_id)) = self.folder_dialog else {
            return;
        };
        let name = self.folder_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.message = Some("A Notes folder name cannot be empty".into());
            cx.notify();
            return;
        }
        let Some(expected_revision) = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.revision)
        else {
            self.folder_dialog = None;
            self.message = Some("That Notes folder is no longer available".into());
            cx.notify();
            return;
        };
        self.folder_dialog = None;
        self.send_action(
            LibraryAction::RenameFolder {
                folder_id,
                expected_revision,
                name,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn begin_folder_delete(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        {
            self.folder_dialog = Some(FolderDialog::Delete(folder_id));
            cx.notify();
        }
    }

    pub(super) fn confirm_folder_delete(&mut self, cx: &mut Context<Self>) {
        let Some(FolderDialog::Delete(folder_id)) = self.folder_dialog else {
            return;
        };
        let Some(expected_revision) = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.revision)
        else {
            self.folder_dialog = None;
            self.message = Some("That Notes folder is no longer available".into());
            cx.notify();
            return;
        };
        self.folder_dialog = None;
        self.send_action(
            LibraryAction::DeleteFolder {
                folder_id,
                expected_revision,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn cancel_folder_dialog(&mut self, cx: &mut Context<Self>) {
        self.folder_dialog = None;
        cx.notify();
    }

    /// Plain ⌫ on the note list, as on the Mac: move the selected note to
    /// Recently Deleted and offer Undo in the status banner. Unlike ⌘⌫
    /// ([`Self::trash_or_restore`]), this never restores an already-deleted
    /// note — Trash has its own permanent-delete commands for that.
    pub(super) fn delete_selected_note_with_undo(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        let note_id = note.id;
        let expected_revision = note.revision;
        self.pending_undo_trash = Some((note_id, expected_revision));
        self.message = Some("Note deleted.".into());
        self.send_action(
            LibraryAction::TrashNote {
                note_id,
                expected_revision,
            },
            cx,
        );
        cx.notify();
        // Undo stays offered for a few seconds, like a Mac toast, then the
        // banner clears itself — unless a newer delete already replaced it.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(6))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pending_undo_trash == Some((note_id, expected_revision)) {
                    this.pending_undo_trash = None;
                    this.message = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The status banner's Undo button after [`Self::delete_selected_note_with_undo`]:
    /// restore the note it just trashed, using the library's current
    /// revision for it (the trash itself already advanced the revision
    /// once).
    pub(super) fn undo_delete_note(&mut self, cx: &mut Context<Self>) {
        let Some((note_id, _)) = self.pending_undo_trash.take() else {
            return;
        };
        let Some(current_revision) = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .notes
                .iter()
                .find(|note| note.id == note_id)
                .map(|note| note.revision)
        }) else {
            self.message = Some("That note is no longer available to restore.".into());
            cx.notify();
            return;
        };
        self.message = None;
        self.send_action(
            LibraryAction::RestoreNote {
                note_id,
                expected_revision: current_revision,
            },
            cx,
        );
    }

    /// File ▸ Duplicate Note (⌘D): a new note with the same title, body,
    /// tags and folder. Attachments are not copied.
    pub(super) fn duplicate_note(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        self.send_action(
            LibraryAction::CreateNote(NewNote {
                created_unix_ms: now_unix_ms(),
                title: note.title.clone(),
                body: note.body.clone(),
                tags: note.tags.clone(),
                folder_id: note.folder_id,
            }),
            cx,
        );
    }

    pub(super) fn trash_or_restore(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        let action = if note.deleted {
            LibraryAction::RestoreNote {
                note_id: note.id,
                expected_revision: note.revision,
            }
        } else {
            LibraryAction::TrashNote {
                note_id: note.id,
                expected_revision: note.revision,
            }
        };
        self.send_action(action, cx);
    }

    pub(super) fn begin_permanent_note_delete(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| note.deleted) else {
            return;
        };
        let note_id = note.id;
        let note_revision = note.revision;
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        let mut attachment_count = 0_usize;
        let mut attachment_bytes = 0_u64;
        for attachment in snapshot
            .attachments
            .iter()
            .filter(|attachment| attachment.note_id == note_id)
        {
            attachment_count = attachment_count.saturating_add(1);
            let Some(total) = attachment_bytes.checked_add(attachment.byte_len) else {
                self.message = Some("The attachment deletion total is too large to review".into());
                cx.notify();
                return;
            };
            attachment_bytes = total;
        }
        self.purge_dialog = Some(PurgeDialog::Note {
            note_id,
            note_revision,
            attachment_count,
            attachment_bytes,
        });
        cx.notify();
    }

    pub(super) fn begin_empty_trash(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        let note_ids = snapshot
            .notes
            .iter()
            .filter(|note| note.deleted)
            .map(|note| note.id)
            .collect::<BTreeSet<_>>();
        if note_ids.is_empty() {
            return;
        }
        let mut attachment_count = 0_usize;
        let mut attachment_bytes = 0_u64;
        for attachment in snapshot
            .attachments
            .iter()
            .filter(|attachment| note_ids.contains(&attachment.note_id))
        {
            attachment_count = attachment_count.saturating_add(1);
            let Some(total) = attachment_bytes.checked_add(attachment.byte_len) else {
                self.message = Some("The Trash deletion total is too large to review".into());
                cx.notify();
                return;
            };
            attachment_bytes = total;
        }
        self.purge_dialog = Some(PurgeDialog::EmptyTrash {
            library_revision: snapshot.revision,
            note_count: note_ids.len(),
            attachment_count,
            attachment_bytes,
        });
        cx.notify();
    }

    pub(super) fn confirm_purge(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.purge_dialog.take() else {
            return;
        };
        let action = match dialog {
            PurgeDialog::Note {
                note_id,
                note_revision,
                ..
            } => LibraryAction::DeleteNotePermanently {
                note_id,
                expected_revision: note_revision,
            },
            PurgeDialog::EmptyTrash {
                library_revision, ..
            } => LibraryAction::EmptyTrash {
                expected_library_revision: library_revision,
            },
        };
        self.send_action(action, cx);
        cx.notify();
    }

    pub(super) fn cancel_purge(&mut self, cx: &mut Context<Self>) {
        self.purge_dialog = None;
        cx.notify();
    }

    pub(super) fn begin_move_note(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        self.move_dialog = Some(MoveDialog {
            note_id: note.id,
            note_revision: note.revision,
            current_folder: note.folder_id,
        });
        cx.notify();
    }

    pub(super) fn move_note_to(&mut self, folder_id: Option<FolderId>, cx: &mut Context<Self>) {
        let Some(dialog) = self.move_dialog.take() else {
            return;
        };
        if dialog.current_folder == folder_id {
            cx.notify();
            return;
        }
        self.send_action(
            LibraryAction::MoveNote {
                note_id: dialog.note_id,
                expected_revision: dialog.note_revision,
                folder_id,
            },
            cx,
        );
        cx.notify();
    }

    pub(super) fn cancel_move_note(&mut self, cx: &mut Context<Self>) {
        self.move_dialog = None;
        cx.notify();
    }

    pub(super) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        self.send_action(
            LibraryAction::SetPinned {
                note_id: note.id,
                expected_revision: note.revision,
                pinned: !note.pinned,
            },
            cx,
        );
    }

    /// The menu bar's live state: View ▸ Sort By ticks the current order,
    /// File ▸ Pin Note says Unpin for a pinned note, and commands that need
    /// a note, the library or no pending change are greyed out without.
    pub(super) fn publish_menu_state(&self, cx: &mut Context<Self>) {
        let ready = self.is_interactive_ready();
        let pending = self.latest_local_generation.is_some();
        let sort_order = self.session.snapshot().map(|snapshot| snapshot.sort_order);
        for (action, order) in [
            ("notes::SortByEdited", SortOrder::Edited),
            ("notes::SortByCreated", SortOrder::Created),
            ("notes::SortByTitle", SortOrder::Title),
        ] {
            rmac_ui::set_menu_checked(action, sort_order == Some(order), cx);
            rmac_ui::set_menu_enabled(action, ready, cx);
        }
        let (has_note, pinned) = self
            .session
            .selected_note()
            .map_or((false, false), |note| (true, note.pinned));
        rmac_ui::set_menu_enabled("notes::TogglePin", ready && has_note, cx);
        rmac_ui::set_menu_label(
            "notes::TogglePin",
            if pinned { "Unpin Note" } else { "Pin Note" },
            cx,
        );
        rmac_ui::set_menu_enabled("notes::DuplicateNote", ready && has_note, cx);
        let body_editable = ready
            && !self.markdown_preview_visible
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted);
        for action in [
            "notes::ToggleBold",
            "notes::ToggleItalic",
            "notes::SetStyleTitle",
            "notes::SetStyleHeading",
            "notes::SetStyleSubheading",
            "notes::SetStyleBody",
            "notes::SetStyleMonospaced",
            "notes::InsertBulletedList",
            "notes::InsertNumberedList",
        ] {
            rmac_ui::set_menu_enabled(action, body_editable, cx);
        }
        rmac_ui::set_menu_enabled("notes::FindInNote", ready && has_note, cx);
        rmac_ui::set_menu_enabled("notes::PrintNote", has_note, cx);
        rmac_ui::set_menu_enabled("notes::ExportNotePdf", has_note, cx);
        rmac_ui::set_menu_enabled(
            "notes::ExportNotes",
            self.session.snapshot().is_some() && !pending,
            cx,
        );
        rmac_ui::set_menu_enabled("notes::ImportNotesBundle", !pending, cx);
        rmac_ui::set_menu_checked(
            "notes::ToggleMarkdownPreview",
            self.markdown_preview_visible,
            cx,
        );
    }

    pub(super) fn set_sort(&mut self, sort_order: SortOrder, cx: &mut Context<Self>) {
        if self.is_interactive_ready() {
            self.send_action(LibraryAction::SetSort(sort_order), cx);
        }
    }

    pub(super) fn accept_migration(&mut self, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(WorkerCommand::AcceptMigration { request_id }, cx);
    }

    pub(super) fn start_empty(&mut self, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(WorkerCommand::StartEmpty { request_id }, cx);
    }

    pub(super) fn retry_pending(&mut self, cx: &mut Context<Self>) {
        self.send(WorkerCommand::RetryPending, cx);
    }
}
