use super::*;

impl NotesView {
    pub(super) fn record_recent_note(&mut self) {
        let selected = self.session.selected_note_id();
        if selected == self.last_editor_note {
            return;
        }
        self.last_editor_note = selected;
        let Some(note_id) = selected else {
            self.recent_position = None;
            return;
        };
        self.recent_notes.retain(|id| *id != note_id);
        self.recent_notes.insert(0, note_id);
        self.recent_notes.truncate(10);
        self.recent_position = Some(0);
    }

    pub(super) fn navigate_recent_note(
        &mut self,
        offset: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(position) = self.recent_position else {
            return;
        };
        let Some(next) = position.checked_add_signed(offset) else {
            return;
        };
        self.open_recent_note(next, window, cx);
    }

    pub(super) fn open_recent_note(
        &mut self,
        position: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note_id) = self.recent_notes.get(position).copied() else {
            return;
        };
        // A note visited in another folder remains reachable from Recents.
        self.session
            .select_folder(rmac_notes_runtime::FolderSelection::All);
        if self.session.select_note(note_id) {
            self.recent_position = Some(position);
            self.last_editor_note = Some(note_id);
            self.sync_editor(window, cx);
            cx.notify();
        }
    }

    pub(super) fn clear_recent_notes(&mut self, cx: &mut Context<Self>) {
        self.recent_notes.clear();
        self.recent_position = None;
        cx.notify();
    }

    pub(super) fn select_folder(
        &mut self,
        folder: rmac_notes_runtime::FolderSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        // Selecting an ordinary folder/All Notes/Trash always drops any
        // Smart Folder filter layered on top of it.
        self.smart_folder_filter = None;
        self.tag_selection = None;
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
        self.note_activity();
        let previous = self.session.selected_note_id();
        if self.session.select_note(note_id) {
            if previous != Some(note_id) || self.latest_local_generation.is_none() {
                self.sync_editor(window, cx);
            }
            cx.notify();
        }
    }

    /// Up/Down in the note list selects the neighbouring note (NOTES-07).
    /// Returns whether the key was used.
    pub(super) fn select_adjacent_note(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_interactive_ready()
            || self.gallery_view
            || self.attachments_browser_visible
            || !self.search_query.read(cx).value().trim().is_empty()
        {
            return false;
        }
        let ids: Vec<NoteId> = self
            .session
            .visible_notes()
            .into_iter()
            .filter(|note| self.note_listed(note))
            .map(|note| note.id)
            .collect();
        if ids.is_empty() {
            return false;
        }
        let current = self
            .session
            .selected_note_id()
            .and_then(|id| ids.iter().position(|candidate| *candidate == id));
        let next = adjacent_index(current, ids.len(), delta);
        if Some(next) != current {
            self.select_note(ids[next], window, cx);
            if !self.group_notes_by_date {
                // Unsectioned rows have a fixed height: keep the new row visible.
                let row_top = LIST_TOP_PADDING + next as f32 * NOTE_ROW_HEIGHT;
                let viewport = f32::from(self.note_list_scroll.bounds().size.height);
                let top = -f32::from(self.note_list_scroll.offset().y);
                let target = if row_top < top {
                    Some(row_top - LIST_TOP_PADDING)
                } else if viewport > 0.0 && row_top + NOTE_ROW_HEIGHT > top + viewport {
                    Some(row_top + NOTE_ROW_HEIGHT - viewport)
                } else {
                    None
                };
                if let Some(target) = target {
                    self.note_list_scroll
                        .set_offset(gpui::point(px(0.0), px(-target.max(0.0))));
                }
            }
        }
        true
    }

    /// The id of the note currently shown in the (single, shared) editor,
    /// used by `quick_note.rs`/`note_window.rs` to decide whether a resume
    /// landed on a real note.
    pub(super) fn selected_note_id(&self) -> Option<NoteId> {
        self.session.selected_note_id()
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
                // Empty, not the literal "New Note": the Mac creates a blank
                // note with the caret ready, and "New Note" is a placeholder
                // rendering only (the list row and `display_title` already
                // fall back to it for an empty title) until the first
                // keystroke supplies real text.
                title: String::new(),
                // Empty: the Mac creates a blank note with the caret ready.
                // Notes ▸ Settings… ▸ New notes start with (NOT-SETTINGS-010,
                // the Mac's Title/Heading/Body picker for the first line's
                // style) is not offered at all — see NOTES-13's row for why
                // neither a real nor a stored-only version of that control
                // works out in Lulo's architecture.
                body: String::new(),
                tags: Vec::new(),
                folder_id,
            }),
            cx,
        );
    }

    /// File ▸ New Quick Note: a new note for the Quick Note window, made
    /// without moving the main window's selection.
    pub(super) fn create_quick_note(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        self.quick_note_id = None;
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let action = LibraryAction::CreateNote(NewNote {
            created_unix_ms: now_unix_ms(),
            title: String::new(),
            body: String::new(),
            tags: Vec::new(),
            folder_id: None,
        });
        match ActionRequest::new(request_id, action) {
            Ok(request) => {
                if self.send(WorkerCommand::Apply(request), cx) {
                    self.quick_note_request = Some((request_id, self.session.selected_note_id()));
                }
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
            }
        }
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
        if self.delete_selected_smart_folder(cx) {
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
        if note.lock.is_some() {
            return;
        }
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
    pub(super) fn publish_menu_state(&self, window: &Window, cx: &mut Context<Self>) {
        rmac_ui::set_menu_label(
            "notes::ToggleFullScreen",
            if window.is_fullscreen() {
                "Exit Full Screen"
            } else {
                "Enter Full Screen"
            },
            cx,
        );
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
        let locked_note = self
            .session
            .selected_note()
            .is_some_and(|note| note.lock.is_some());
        rmac_ui::set_menu_enabled("notes::TogglePin", ready && has_note, cx);
        rmac_ui::set_menu_label(
            "notes::TogglePin",
            if pinned { "Unpin Note" } else { "Pin Note" },
            cx,
        );
        // A locked note is never copied into a plaintext note.
        rmac_ui::set_menu_enabled(
            "notes::DuplicateNote",
            ready && has_note && !locked_note,
            cx,
        );
        rmac_ui::set_menu_enabled("notes::ToggleLightBackground", ready && has_note, cx);
        rmac_ui::set_menu_checked(
            "notes::ToggleLightBackground",
            self.session
                .selected_note()
                .is_some_and(|note| self.light_background_notes.contains(&note.id)),
            cx,
        );
        let body_editable = ready
            && !self.markdown_preview_visible
            && !self.selected_note_closed()
            && self
                .session
                .selected_note()
                .is_some_and(|note| !note.deleted);
        let body_focused = self.body_focused(window, cx);
        let body_has_selection = !self.body_selection(cx).is_empty();
        for action in [
            "notes::ToggleBold",
            "notes::ToggleItalic",
            "notes::ToggleStrikethrough",
            "notes::SetStyleTitle",
            "notes::SetStyleHeading",
            "notes::SetStyleSubheading",
            "notes::SetStyleBody",
            "notes::SetStyleMonospaced",
            "notes::InsertBulletedList",
            "notes::InsertDashedList",
            "notes::InsertNumberedList",
            "notes::InsertBlockQuote",
            "notes::ToggleChecklistDone",
            "notes::InsertLink",
            "notes::IncreaseIndent",
            "notes::DecreaseIndent",
            "notes::PastePlainText",
            "notes::TickAll",
            "notes::UntickAll",
            "notes::MoveTickedToBottom",
            "notes::DeleteTicked",
            "notes::MoveItemUp",
            "notes::MoveItemDown",
            "notes::InsertTable",
        ] {
            rmac_ui::set_menu_enabled(action, body_editable && body_focused, cx);
        }
        rmac_ui::set_menu_enabled(
            "notes::ConvertToText",
            body_editable && body_focused && self.current_line_has_structure(cx),
            cx,
        );
        rmac_ui::set_menu_enabled("notes::DeleteSelectedNote", ready && has_note, cx);
        rmac_ui::set_menu_enabled(
            "notes::RenameSelectedFolder",
            ready
                && matches!(
                    self.session.folder_selection(),
                    rmac_notes_runtime::FolderSelection::Folder(_)
                ),
            cx,
        );
        rmac_ui::set_menu_enabled("notes::FindInNoteNext", ready && has_note, cx);
        rmac_ui::set_menu_enabled("notes::FindInNotePrevious", ready && has_note, cx);
        rmac_ui::set_menu_enabled("notes::UseSelectionForFind", body_editable, cx);
        rmac_ui::set_menu_enabled(
            "notes::JumpToSelection",
            body_editable && body_has_selection,
            cx,
        );
        rmac_ui::set_menu_enabled("notes::FindInNote", ready && has_note, cx);
        for action in [
            "notes::MakeUppercase",
            "notes::MakeLowercase",
            "notes::Capitalise",
        ] {
            rmac_ui::set_menu_enabled(
                action,
                body_editable && body_focused && body_has_selection,
                cx,
            );
        }
        rmac_ui::set_menu_enabled("notes::FindAndReplace", body_editable, cx);
        // Edit ▸ Writing Tools: the body's selection, or the whole note.
        rmac_writing_tools::publish_menu_state(body_editable, cx);
        rmac_ui::set_menu_enabled(
            "notes::PrintNote",
            has_note && !self.selected_note_closed(),
            cx,
        );
        rmac_ui::set_menu_enabled("notes::ExportNotePdf", has_note, cx);
        rmac_ui::set_menu_enabled(
            "notes::ExportNoteMarkdown",
            ready
                && !pending
                && self
                    .session
                    .selected_note()
                    .is_some_and(|note| !note.deleted && note.attachments.is_empty()),
            cx,
        );
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
        rmac_ui::set_menu_label(
            "notes::ToggleFolders",
            if self.folders_shown() {
                "Hide Folders"
            } else {
                "Show Folders"
            },
            cx,
        );
        rmac_ui::set_menu_label(
            "notes::ToggleNoteCount",
            if self.show_note_count {
                "Hide Note Count"
            } else {
                "Show Note Count"
            },
            cx,
        );
        rmac_ui::set_menu_label(
            "notes::ToggleToolbar",
            if self.toolbar_visible {
                "Hide Toolbar"
            } else {
                "Show Toolbar"
            },
            cx,
        );
        rmac_ui::set_menu_checked("notes::ShowListView", !self.gallery_view, cx);
        rmac_ui::set_menu_checked("notes::ShowGalleryView", self.gallery_view, cx);
        rmac_ui::set_menu_label(
            "notes::ToggleAttachmentsBrowser",
            if self.attachments_browser_visible {
                "Hide Attachments Browser"
            } else {
                "Show Attachments Browser"
            },
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::ShowAttachmentInNote",
            self.attachments_browser_visible && self.selected_attachment.is_some(),
            cx,
        );
        rmac_ui::set_menu_enabled("notes::ZoomIn", self.note_zoom < 12, cx);
        rmac_ui::set_menu_enabled("notes::ZoomOut", self.note_zoom > -5, cx);
        rmac_ui::set_menu_enabled("notes::ZoomReset", self.note_zoom != 0, cx);
        let visible_sections = self.visible_sections(cx);
        let selected_section = self
            .selected_section()
            .filter(|section| visible_sections.contains(section));
        let selected_collapsed = selected_section
            .as_ref()
            .is_some_and(|section| self.collapsed_sections.contains(section));
        rmac_ui::set_menu_enabled(
            "notes::CollapseSection",
            selected_section.is_some() && !selected_collapsed,
            cx,
        );
        rmac_ui::set_menu_enabled("notes::ExpandSection", selected_collapsed, cx);
        rmac_ui::set_menu_enabled(
            "notes::CollapseAllSections",
            visible_sections
                .iter()
                .any(|section| !self.collapsed_sections.contains(section)),
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::ExpandAllSections",
            visible_sections
                .iter()
                .any(|section| self.collapsed_sections.contains(section)),
            cx,
        );
        let previous = self
            .recent_position
            .is_some_and(|position| position + 1 < self.recent_notes.len());
        let next = self.recent_position.is_some_and(|position| position > 0);
        rmac_ui::set_menu_enabled("notes::PreviousRecentNote", ready && previous, cx);
        rmac_ui::set_menu_enabled("notes::NextRecentNote", ready && next, cx);
        rmac_ui::set_menu_enabled("notes::ClearRecentNotes", !self.recent_notes.is_empty(), cx);
        let mut recent_items = vec![
            rmac_ui::MenuItem::new("Previous Note", "notes::PreviousRecentNote", "⌥⌘[")
                .enabled(ready && previous),
            rmac_ui::MenuItem::new("Next Note", "notes::NextRecentNote", "⌥⌘]")
                .enabled(ready && next),
        ];
        if let Some(snapshot) = self.session.snapshot() {
            for (index, note_id) in self.recent_notes.iter().enumerate() {
                let Some(note) = snapshot
                    .notes
                    .iter()
                    .find(|note| note.id == *note_id && !note.deleted)
                else {
                    continue;
                };
                let title: String = display_title(&note.title)
                    .chars()
                    .map(|ch| if ch.is_control() { ' ' } else { ch })
                    .collect();
                let title = if title.len() > 64 {
                    let mut end = 61;
                    while !title.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &title[..end])
                } else {
                    title
                };
                recent_items.push(rmac_ui::MenuItem::new(
                    title,
                    format!("notes::OpenRecentNote{index}"),
                    "",
                ));
            }
        }
        recent_items.push(
            rmac_ui::MenuItem::new("Clear Menu", "notes::ClearRecentNotes", "")
                .enabled(!self.recent_notes.is_empty())
                .separated(),
        );
        rmac_ui::set_menu_children("notes::RecentNotesMenu", recent_items, cx);

        for action in [
            "notes::ToggleUnderline",
            "notes::ToggleHighlight",
            "notes::ToggleSuperscript",
            "notes::ToggleSubscript",
            "notes::BaselineUseDefault",
            "notes::RemoveStyle",
            "notes::AlignLeft",
            "notes::AlignCentre",
            "notes::AlignJustify",
            "notes::AlignRight",
            "notes::ParagraphDirectionDefault",
            "notes::ParagraphDirectionLeftToRight",
            "notes::ParagraphDirectionRightToLeft",
            "notes::SelectionDirectionDefault",
            "notes::SelectionDirectionLeftToRight",
            "notes::SelectionDirectionRightToLeft",
            "notes::FontBigger",
            "notes::FontSmaller",
            "notes::ShowFonts",
            "notes::ShowColours",
        ] {
            rmac_ui::set_menu_enabled(action, body_editable && body_focused, cx);
        }
        // The Mac's section labels inside Writing Direction.
        rmac_ui::set_menu_enabled("notes::WritingDirectionParagraphLabel", false, cx);
        rmac_ui::set_menu_enabled("notes::WritingDirectionSelectionLabel", false, cx);
        let paragraph = self.body.read(cx).paragraph_style_at_selection();
        let char_style = self.body.read(cx).style_at_selection();
        for (action, direction) in [
            ("notes::ParagraphDirectionDefault", WritingDirection::Default),
            (
                "notes::ParagraphDirectionLeftToRight",
                WritingDirection::LeftToRight,
            ),
            (
                "notes::ParagraphDirectionRightToLeft",
                WritingDirection::RightToLeft,
            ),
        ] {
            rmac_ui::set_menu_checked(action, paragraph.direction == direction, cx);
        }
        for (action, direction) in [
            ("notes::SelectionDirectionDefault", WritingDirection::Default),
            (
                "notes::SelectionDirectionLeftToRight",
                WritingDirection::LeftToRight,
            ),
            (
                "notes::SelectionDirectionRightToLeft",
                WritingDirection::RightToLeft,
            ),
        ] {
            rmac_ui::set_menu_checked(action, char_style.embedding == direction, cx);
        }
        for (action, role) in [
            ("notes::SetStyleTitle", ParagraphRole::Title),
            ("notes::SetStyleHeading", ParagraphRole::Heading),
            ("notes::SetStyleSubheading", ParagraphRole::Subheading),
            ("notes::SetStyleBody", ParagraphRole::Body),
            ("notes::SetStyleMonospaced", ParagraphRole::Monostyled),
        ] {
            rmac_ui::set_menu_checked(action, body_focused && paragraph.role == role, cx);
        }
        for (action, list) in [
            ("notes::InsertBulletedList", ListKind::Bullet),
            ("notes::InsertDashedList", ListKind::Hyphen),
            ("notes::InsertNumberedList", ListKind::Numbered),
            ("notes::InsertChecklist", ListKind::Checkbox),
        ] {
            rmac_ui::set_menu_checked(action, body_focused && paragraph.list == Some(list), cx);
        }
        rmac_ui::set_menu_checked("notes::InsertBlockQuote", body_focused && paragraph.quote, cx);
        rmac_ui::set_menu_label(
            "notes::ToggleChecklistDone",
            if paragraph.list == Some(ListKind::Checkbox) && paragraph.checked {
                "Mark as Unticked"
            } else {
                "Mark as Ticked"
            },
            cx,
        );
        rmac_ui::set_menu_checked("notes::ToggleBold", body_focused && char_style.bold, cx);
        rmac_ui::set_menu_checked("notes::ToggleItalic", body_focused && char_style.italic, cx);
        rmac_ui::set_menu_checked(
            "notes::ToggleUnderline",
            body_focused && char_style.underline,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::ToggleStrikethrough",
            body_focused && char_style.strikethrough,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::ToggleHighlight",
            body_focused && char_style.highlight.is_some(),
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::CopyStyle",
            body_editable && body_focused && body_has_selection,
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::PasteStyle",
            body_editable && body_focused && self.copied_style.is_some(),
            cx,
        );
        rmac_ui::set_menu_enabled("notes::PasteAndRetainStyle", body_editable, cx);
        let alignment = self.current_alignment(cx);
        for (action, value) in [
            ("notes::AlignLeft", Alignment::Left),
            ("notes::AlignCentre", Alignment::Center),
            ("notes::AlignJustify", Alignment::Justified),
            ("notes::AlignRight", Alignment::Right),
        ] {
            rmac_ui::set_menu_checked(action, alignment == value, cx);
        }
        rmac_ui::set_menu_checked(
            "notes::MathsResultsOff",
            self.maths_results_mode == MathsResultsMode::Off,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::MathsResultsSuggest",
            self.maths_results_mode == MathsResultsMode::SuggestResults,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::MathsResultsInsert",
            self.maths_results_mode == MathsResultsMode::InsertResults,
            cx,
        );
        rmac_ui::set_menu_label(
            "notes::ToggleShowHighlights",
            if self.show_highlights {
                "Hide Highlights"
            } else {
                "Show Highlights"
            },
            cx,
        );
        rmac_ui::set_menu_checked("notes::ToggleShowHighlights", self.show_highlights, cx);
        let (locked, deleted) = self
            .session
            .selected_note()
            .map_or((false, false), |note| (note.lock.is_some(), note.deleted));
        rmac_ui::set_menu_enabled("notes::ToggleLockNote", ready && has_note && !deleted, cx);
        rmac_ui::set_menu_label(
            "notes::ToggleLockNote",
            if locked { "Remove Lock" } else { "Lock Note" },
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::CloseAllLockedNotes",
            self.session.any_locked_note_open(),
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::OpenNoteInNewWindow",
            ready && has_note && !locked && !deleted,
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::RecordAudio",
            ready && has_note && !locked && !deleted,
            cx,
        );
        rmac_ui::set_menu_label(
            "notes::RecordAudio",
            if self.recording.is_some() {
                "Stop Recording"
            } else {
                "Record Audio…"
            },
            cx,
        );
        rmac_ui::set_menu_enabled("notes::CreateSmartFolder", ready, cx);
        rmac_ui::set_menu_enabled(
            "notes::CreateSmartFolderFromSelection",
            ready
                && (matches!(
                    self.tag_selection,
                    Some(tag_browser::TagSelection::Tags(_))
                ) || !self.tags.read(cx).selected_range().is_empty()),
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::AttachFile",
            ready
                && has_note
                && !pending
                && self.session.selected_note().is_some_and(|n| !n.deleted),
            cx,
        );
        rmac_ui::set_menu_enabled(
            "notes::RenameAttachment",
            ready && self.selected_attachment.is_some(),
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

    /// The selected note is locked and its password is not open: only the
    /// lock is shown, nothing of its content.
    pub(super) fn selected_note_closed(&self) -> bool {
        self.session
            .selected_note()
            .is_some_and(|note| self.session.is_note_closed(note))
    }

    /// Any activity inside Notes keeps open locked notes open for another
    /// `LOCKED_NOTES_IDLE_TIMEOUT`.
    pub(super) fn note_activity(&mut self) {
        self.last_activity = std::time::Instant::now();
    }

    fn clear_lock_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            self.lock_password_input.clone(),
            self.lock_verify_input.clone(),
            self.lock_hint_input.clone(),
            self.lock_old_password_input.clone(),
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn open_lock_dialog(
        &mut self,
        dialog: LockDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_lock_inputs(window, cx);
        self.lock_dialog = Some(dialog);
        self.lock_dialog_error = None;
        self.lock_request_id = None;
        self.lock_password_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// File ▸ Lock Note / Remove Lock (one menu item, relabelled like the
    /// Mac's). Locking with the password open needs no prompt; the first
    /// lock creates the password; otherwise Notes asks for it.
    pub(super) fn toggle_lock_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        if note.deleted {
            return;
        }
        let (note_id, revision, locked) = (note.id, note.revision, note.lock.is_some());
        self.note_activity();
        if locked {
            if self.session.is_note_open(note_id) {
                self.send_lock_action(
                    LibraryAction::RemoveNoteLock {
                        note_id,
                        expected_revision: revision,
                        password: None,
                    },
                    cx,
                );
            } else {
                self.open_lock_dialog(LockDialog::RemoveLock { note_id, revision }, window, cx);
            }
            return;
        }
        match self
            .session
            .snapshot()
            .and_then(|snapshot| snapshot.current_lock_key)
        {
            None => self.open_lock_dialog(
                LockDialog::CreatePassword {
                    then_lock: Some((note_id, revision)),
                },
                window,
                cx,
            ),
            Some(current) if self.keyring.contains(current) => self.send_lock_action(
                LibraryAction::LockNote {
                    note_id,
                    expected_revision: revision,
                    credential: LockCredential::Open,
                },
                cx,
            ),
            Some(_) => self.open_lock_dialog(
                LockDialog::LockWithPassword { note_id, revision },
                window,
                cx,
            ),
        }
    }

    fn send_lock_action(&mut self, action: LibraryAction, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let Ok(request) = ActionRequest::new(request_id, action) else {
            return;
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.lock_request_id = Some(request_id);
        }
        cx.notify();
    }

    /// The locked-note placeholder's "View Note…" button.
    pub(super) fn request_unlock(
        &mut self,
        note_id: NoteId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let closed = self
            .session
            .snapshot()
            .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
            .is_some_and(|note| self.session.is_note_closed(note));
        if closed {
            self.open_lock_dialog(LockDialog::Unlock(note_id), window, cx);
        }
    }

    /// Application ▸ Close All Locked Notes (and inactivity, sleep, the lock
    /// screen): the worker commits the open note's last edit, then forgets
    /// every key; the view re-renders every locked note closed.
    pub(super) fn close_all_locked_notes(&mut self, cx: &mut Context<Self>) {
        self.idle_lock_timer = None;
        if !self.session.any_locked_note_open() && self.keyring.is_empty() {
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(WorkerCommand::CloseLockedNotes { request_id }, cx);
        cx.notify();
    }

    /// Start (once) the inactivity timer while any locked note is open. It
    /// wakes only at its deadline, never polls.
    pub(super) fn arm_idle_lock(&mut self, cx: &mut Context<Self>) {
        if !self.session.any_locked_note_open() {
            self.idle_lock_timer = None;
            return;
        }
        if self.idle_lock_timer.is_some() {
            return;
        }
        self.idle_lock_timer = Some(cx.spawn(async move |this, cx| loop {
            let Ok(remaining) = this.update(cx, |this, _| {
                LOCKED_NOTES_IDLE_TIMEOUT.saturating_sub(this.last_activity.elapsed())
            }) else {
                return;
            };
            if remaining.is_zero() {
                let _ = this.update(cx, |this, cx| {
                    this.idle_lock_timer = None;
                    this.close_all_locked_notes(cx);
                });
                return;
            }
            cx.background_executor().timer(remaining).await;
        }));
    }

    /// Notes ▸ Settings… ▸ Change Password… (or, with no password yet, the
    /// first password). Opened from the Settings window, which has no
    /// handle on this window's inputs, so the fields are cleared when the
    /// dialog closes instead.
    pub(super) fn begin_change_password(&mut self, cx: &mut Context<Self>) {
        let has_password = self
            .session
            .snapshot()
            .is_some_and(|snapshot| snapshot.current_lock_key.is_some());
        self.lock_dialog = Some(if has_password {
            LockDialog::ChangePassword
        } else {
            LockDialog::CreatePassword { then_lock: None }
        });
        self.lock_dialog_error = None;
        self.lock_request_id = None;
        cx.notify();
    }

    /// Notes ▸ Settings… ▸ Reset Password…: confirmation first.
    pub(super) fn begin_reset_password(&mut self, cx: &mut Context<Self>) {
        self.lock_dialog = Some(LockDialog::ConfirmReset);
        self.lock_dialog_error = None;
        self.lock_request_id = None;
        cx.notify();
    }

    pub(super) fn cancel_lock_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lock_dialog = None;
        self.lock_dialog_error = None;
        self.lock_request_id = None;
        self.clear_lock_inputs(window, cx);
        cx.notify();
    }

    /// A password request finished: close its dialog and wipe the fields.
    pub(super) fn finish_lock_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lock_request_id = None;
        self.lock_dialog = None;
        self.lock_dialog_error = None;
        self.clear_lock_inputs(window, cx);
    }

    /// A password request failed: keep the dialog, say why, and show the
    /// hint after a wrong password (as macOS does).
    pub(super) fn fail_lock_request(&mut self, failure: WorkerFailure, cx: &mut Context<Self>) {
        self.lock_request_id = None;
        let hint = self.lock_dialog.and_then(|dialog| {
            let snapshot = self.session.snapshot()?;
            let key_id = match dialog {
                LockDialog::Unlock(note_id) | LockDialog::RemoveLock { note_id, .. } => {
                    snapshot
                        .notes
                        .iter()
                        .find(|note| note.id == note_id)?
                        .lock
                        .as_ref()?
                        .key_id
                }
                _ => snapshot.current_lock_key?,
            };
            let hint = snapshot.lock_key(key_id)?.hint.clone();
            (!hint.is_empty()).then_some(hint)
        });
        let message = worker_failure_message(failure);
        self.lock_dialog_error = Some(match (failure, hint) {
            (WorkerFailure::Lock(LockError::WrongPassword), Some(hint)) => {
                format!("{message} Hint: {hint}").into()
            }
            _ => message.into(),
        });
        if self.lock_dialog.is_none() {
            self.message = self.lock_dialog_error.take();
        }
        cx.notify();
    }

    fn lock_field(&self, input: &Entity<InputState>, cx: &Context<Self>) -> LockSecret {
        LockSecret::new(input.read(cx).value().to_string())
    }

    pub(super) fn confirm_lock_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.lock_dialog else {
            return;
        };
        if self.lock_request_id.is_some() {
            return;
        }
        self.note_activity();
        let password = self.lock_field(&self.lock_password_input, cx);
        let verify = self.lock_field(&self.lock_verify_input, cx);
        let hint = self.lock_hint_input.read(cx).value().trim().to_string();
        let new_password_error = if password.is_empty() {
            Some("Enter a password.")
        } else if password.as_str() != verify.as_str() {
            Some("The passwords don’t match.")
        } else if !hint.is_empty() && hint == password.as_str() {
            Some("The hint can’t be the password.")
        } else {
            None
        };
        match dialog {
            LockDialog::CreatePassword { then_lock } => {
                if let Some(error) = new_password_error {
                    self.lock_dialog_error = Some(error.into());
                    cx.notify();
                    return;
                }
                let action = match then_lock {
                    Some((note_id, revision)) => LibraryAction::LockNote {
                        note_id,
                        expected_revision: revision,
                        credential: LockCredential::NewPassword { password, hint },
                    },
                    None => LibraryAction::ResetLockPassword {
                        new_password: password,
                        hint,
                    },
                };
                self.send_lock_action(action, cx);
            }
            LockDialog::LockWithPassword { note_id, revision } => {
                if password.is_empty() {
                    return;
                }
                self.send_lock_action(
                    LibraryAction::LockNote {
                        note_id,
                        expected_revision: revision,
                        credential: LockCredential::Password(password),
                    },
                    cx,
                );
            }
            LockDialog::Unlock(note_id) => {
                if password.is_empty() {
                    return;
                }
                let Some(request_id) = self.take_request_id() else {
                    return;
                };
                if self.send(
                    WorkerCommand::UnlockNotes {
                        request_id,
                        note_id,
                        password,
                    },
                    cx,
                ) {
                    self.lock_request_id = Some(request_id);
                }
            }
            LockDialog::RemoveLock { note_id, revision } => {
                if password.is_empty() {
                    return;
                }
                self.send_lock_action(
                    LibraryAction::RemoveNoteLock {
                        note_id,
                        expected_revision: revision,
                        password: Some(password),
                    },
                    cx,
                );
            }
            LockDialog::ChangePassword => {
                let old_password = self.lock_field(&self.lock_old_password_input, cx);
                if old_password.is_empty() {
                    self.lock_dialog_error = Some("Enter the old password.".into());
                    cx.notify();
                    return;
                }
                if let Some(error) = new_password_error {
                    self.lock_dialog_error = Some(error.into());
                    cx.notify();
                    return;
                }
                self.send_lock_action(
                    LibraryAction::ChangeLockPassword {
                        old_password,
                        new_password: password,
                        hint,
                    },
                    cx,
                );
            }
            LockDialog::ConfirmReset => {
                self.open_lock_dialog(LockDialog::ResetPassword, window, cx);
            }
            LockDialog::ResetPassword => {
                if let Some(error) = new_password_error {
                    self.lock_dialog_error = Some(error.into());
                    cx.notify();
                    return;
                }
                self.send_lock_action(
                    LibraryAction::ResetLockPassword {
                        new_password: password,
                        hint,
                    },
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// File ▸ New Smart Folder: a tag collection stored with the library,
    /// named through the same dialog New Smart Folder with Tag Selection
    /// prefills from the current tag selection.
    pub(super) fn begin_create_smart_folder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.smart_folder_name_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.smart_folder_dialog = Some(String::new());
        cx.notify();
    }

    pub(super) fn begin_create_smart_folder_from_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The Tags browser's chosen tag, or else the text selected in the
        // Tags field.
        let browser_tag = match &self.tag_selection {
            Some(tag_browser::TagSelection::Tags(chosen)) => self
                .library_tags()
                .into_iter()
                .find(|tag| chosen.contains(&tag.to_lowercase())),
            _ => None,
        };
        let prefill = match browser_tag {
            Some(tag) => tag,
            None => {
                let tags = self.tags.read(cx);
                let selection = tags.selected_range();
                let value = tags.value();
                let Some(selected) = value.get(selection).filter(|text| !text.is_empty()) else {
                    return;
                };
                selected.trim().trim_start_matches('#').to_string()
            }
        };
        self.smart_folder_name_input.update(cx, |input, cx| {
            input.set_value(prefill.clone(), window, cx);
        });
        self.smart_folder_dialog = Some(prefill);
        cx.notify();
    }

    pub(super) fn cancel_smart_folder_dialog(&mut self, cx: &mut Context<Self>) {
        self.smart_folder_dialog = None;
        cx.notify();
    }

    pub(super) fn commit_smart_folder(&mut self, cx: &mut Context<Self>) {
        if self.smart_folder_dialog.is_none() || !self.is_interactive_ready() {
            return;
        }
        let name = self
            .smart_folder_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }
        let tag = name.trim_start_matches('#').trim().to_string();
        self.smart_folder_dialog = None;
        self.send_action(LibraryAction::CreateSmartFolder { name, tag }, cx);
        cx.notify();
    }

    /// Delete the selected Smart Folder (File ▸ Delete Folder while it is
    /// selected). Its notes are untouched: a Smart Folder only collects.
    pub(super) fn delete_selected_smart_folder(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(smart_folder_id) = self.smart_folder_filter else {
            return false;
        };
        if !self.is_interactive_ready() {
            return true;
        }
        self.smart_folder_filter = None;
        self.send_action(LibraryAction::DeleteSmartFolder { smart_folder_id }, cx);
        cx.notify();
        true
    }

    /// Selecting a Smart Folder in the sidebar: narrow the note list to
    /// notes tagged with its one tag, on top of the ordinary All Notes
    /// selection (Smart Folders have no folder membership of their own).
    pub(super) fn select_smart_folder(
        &mut self,
        id: SmartFolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_folder(rmac_notes_runtime::FolderSelection::All, window, cx);
        self.smart_folder_filter = Some(id);
        cx.notify();
    }

    pub(super) fn set_maths_results_mode(
        &mut self,
        mode: MathsResultsMode,
        cx: &mut Context<Self>,
    ) {
        self.maths_results_mode = mode;
        cx.notify();
    }

    /// Format ▸ Maths Results after a change to the body: typing `=` after
    /// an expression at the end of a line inserts its result (Insert
    /// Results) or offers it (Suggest Results). Only a change that grew the
    /// body by the typed `=` counts, so deleting back to an `=` never
    /// computes again; any other change withdraws an offer.
    pub(super) fn update_maths_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let body = self.body.read(cx);
            (body.text(), body.selected_range())
        };
        let previous = std::mem::replace(&mut self.maths_prev_len, text.len());
        self.maths_suggestion = None;
        if self.maths_results_mode == MathsResultsMode::Off
            || !self.body_format_editable()
            || cursor.start != cursor.end
            || text.len() != previous + 1
            || !text[..cursor.end].ends_with('=')
            || text[cursor.end..].chars().next().is_some_and(|c| c != '\n')
        {
            return;
        }
        let line_start = text[..cursor.end].rfind('\n').map_or(0, |i| i + 1);
        let Some(result) = crate::maths_results::result_for_line(&text[line_start..cursor.end])
        else {
            return;
        };
        match self.maths_results_mode {
            MathsResultsMode::InsertResults => {
                let at = cursor.end;
                self.body
                    .update(cx, |editor, cx| editor.replace_range(at..at, &result, cx));
            }
            MathsResultsMode::SuggestResults => {
                self.maths_suggestion = Some(crate::maths_results::Suggestion {
                    at: cursor.end,
                    result,
                });
            }
            MathsResultsMode::Off => {}
        }
        let _ = window;
        cx.notify();
    }

    /// Return or Tab while a result is offered: insert it.
    pub(super) fn accept_maths_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(suggestion) = self.maths_suggestion.take() else {
            return false;
        };
        let caret = self.body_selection(cx);
        if caret != (suggestion.at..suggestion.at) || !self.body_format_editable() {
            cx.notify();
            return false;
        }
        self.body.update(cx, |editor, cx| {
            editor.replace_range(suggestion.at..suggestion.at, &suggestion.result, cx)
        });
        cx.notify();
        true
    }

    pub(super) fn toggle_show_highlights(&mut self, cx: &mut Context<Self>) {
        self.show_highlights = !self.show_highlights;
        let show = self.show_highlights;
        self.body
            .update(cx, |editor, cx| editor.set_show_highlights(show, cx));
        cx.notify();
    }

    pub(super) fn toggle_customise_toolbar(&mut self, cx: &mut Context<Self>) {
        self.customise_toolbar_open = !self.customise_toolbar_open;
        cx.notify();
    }

    pub(super) fn toggle_hidden_toolbar_item(&mut self, item: ToolbarItem, cx: &mut Context<Self>) {
        if !self.hidden_toolbar_items.insert(item) {
            self.hidden_toolbar_items.remove(&item);
        }
        cx.notify();
    }

    pub(super) fn show_smart_folders_help(&mut self, cx: &mut Context<Self>) {
        self.notes_help = Some(
            "A Smart Folder keeps every note with one tag together and updates itself as you tag \
             and untag notes. Choose File ▸ New Smart Folder, or select a tag first and choose \
             File ▸ More ▸ New Smart Folder with Tag Selection.",
        );
        cx.notify();
    }

    pub(super) fn show_tags_help(&mut self, cx: &mut Context<Self>) {
        self.notes_help = Some(
            "Add a tag to a note in its Tags field. Select a tag's text and choose File ▸ More ▸ \
             New Smart Folder with Tag Selection to collect every note that shares it.",
        );
        cx.notify();
    }

    pub(super) fn dismiss_notes_help(&mut self, cx: &mut Context<Self>) {
        self.notes_help = None;
        cx.notify();
    }
}

/// Index reached by Up/Down from `current` in a list of `len` rows, clamped
/// at both ends like the Mac's list.
fn adjacent_index(current: Option<usize>, len: usize, delta: isize) -> usize {
    match current {
        Some(index) => index.saturating_add_signed(delta).min(len - 1),
        None if delta > 0 => 0,
        None => len - 1,
    }
}

#[cfg(test)]
mod adjacent_tests {
    use super::adjacent_index;

    #[test]
    fn arrows_clamp_at_the_ends() {
        assert_eq!(adjacent_index(Some(0), 3, -1), 0);
        assert_eq!(adjacent_index(Some(0), 3, 1), 1);
        assert_eq!(adjacent_index(Some(2), 3, 1), 2);
        assert_eq!(adjacent_index(None, 3, 1), 0);
        assert_eq!(adjacent_index(None, 3, -1), 2);
    }
}
