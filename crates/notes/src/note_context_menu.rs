//! The note list's and folder sidebar's right-click context menus
//! (right-clicking a note or folder previously did nothing). Every item
//! here dispatches a real action the File/Edit menus or the toolbar's
//! "More" button already wire up — this module only decides which of
//! those existing commands apply to the row under the cursor, matching
//! macOS 26 Notes' own right-click menus.

use super::*;

/// Which row a currently-open context menu was opened for; only meaningful
/// while `NotesView::context_menu` is `Some`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContextMenuTarget {
    /// The note list's per-note menu, acting on `session.selected_note()`
    /// (the row under the cursor, selected first if it wasn't already, as
    /// on the Mac).
    Note,
    /// A regular folder's sidebar menu, acting on that folder (selected
    /// first, as above).
    Folder(FolderId),
}

/// The note-list context menu's item labels, in order, for a note with the
/// given pinned/locked/deleted state. `build_note_context_menu` builds the
/// real menu from the same three flags; this pure projection lets a test
/// catch a relabelling or a dropped item without constructing a
/// `ContextMenu`/`ContextMenuState` (which need a live `Window`/`App`).
#[cfg(test)]
pub(super) fn note_context_menu_labels(
    pinned: bool,
    locked: bool,
    deleted: bool,
) -> Vec<&'static str> {
    if deleted {
        // Recently Deleted: the Mac offers only recovery or a confirmed
        // permanent delete, not Pin/Lock/Move/Duplicate.
        return vec!["Put Back", "Delete Immediately…"];
    }
    vec![
        if pinned { "Unpin Note" } else { "Pin Note" },
        if locked { "Remove Lock" } else { "Lock Note" },
        "Move to",
        "Duplicate",
        "Delete",
        "Open Note in New Window",
    ]
}

/// The folder-sidebar context menu's item labels, in order. Unlike the note
/// menu this has no state-dependent relabelling, but the test still pins
/// the exact wording and order against drift.
#[cfg(test)]
pub(super) fn folder_context_menu_labels() -> Vec<&'static str> {
    vec![
        "New Folder",
        "Rename Folder…",
        "Delete Folder…",
        "Sort Notes By",
    ]
}

impl NotesView {
    /// Right mouse-down on a note-list row: select it first if it wasn't
    /// already selected (the Mac's own behaviour), then open the menu at
    /// the click position.
    pub(super) fn open_note_context_menu(
        &mut self,
        note_id: NoteId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.session.selected_note_id() != Some(note_id) {
            if self.search_query.read(cx).value().trim().is_empty() {
                self.select_note(note_id, window, cx);
            } else {
                self.select_search_result(note_id, window, cx);
            }
        }
        self.context_menu_target = ContextMenuTarget::Note;
        self.context_menu = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    /// Right mouse-down on a folder-sidebar row: select the folder first
    /// (the "Folder Actions" popup button already requires a selected
    /// folder; the context menu's Rename/Delete act on that same
    /// selection), then open the menu.
    pub(super) fn open_folder_context_menu(
        &mut self,
        folder_id: FolderId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.session.folder_selection() != rmac_notes_runtime::FolderSelection::Folder(folder_id)
        {
            self.select_folder(
                rmac_notes_runtime::FolderSelection::Folder(folder_id),
                window,
                cx,
            );
        }
        self.context_menu_target = ContextMenuTarget::Folder(folder_id);
        self.context_menu = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    /// Moves `session.selected_note()` straight to `folder_id` (`None` for
    /// All Notes), bypassing the Move Note… dialog: the `Move to` submenu
    /// already names the destination, so there is nothing left to pick.
    /// Reuses the exact `LibraryAction::MoveNote` the dialog's own
    /// `move_note_to` sends.
    pub(super) fn move_selected_note_to(
        &mut self,
        folder_id: Option<FolderId>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        if note.folder_id == folder_id {
            return;
        }
        self.send_action(
            LibraryAction::MoveNote {
                note_id: note.id,
                expected_revision: note.revision,
                folder_id,
            },
            cx,
        );
        cx.notify();
    }

    /// Builds the open context menu's content, if any. Rendered as the
    /// LAST child of the app root (see `root_presentation::render_root`).
    pub(super) fn render_context_menu(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.context_menu.as_ref()?;
        let pos = state.position();
        let menu = match self.context_menu_target {
            ContextMenuTarget::Note => {
                let note = self.session.selected_note()?;
                let folders = self.session.folders();
                build_note_context_menu(
                    pos,
                    note.pinned,
                    note.lock.is_some(),
                    note.deleted,
                    note.folder_id,
                    &folders,
                )
            }
            ContextMenuTarget::Folder(_) => {
                let sort_order = self.session.snapshot().map(|snapshot| snapshot.sort_order);
                build_folder_context_menu(pos, sort_order)
            }
        };
        Some(menu.render(state).into_any_element())
    }
}

fn build_note_context_menu(
    pos: Point<Pixels>,
    pinned: bool,
    locked: bool,
    deleted: bool,
    current_folder: Option<FolderId>,
    folders: &[&rmac_notes_store::FolderRecord],
) -> rmac_ui::ContextMenu {
    if deleted {
        return rmac_ui::ContextMenu::new(pos)
            .item("Put Back", Box::new(TrashOrRestore))
            .separator()
            .danger_item("Delete Immediately…", Box::new(DeleteNotePermanently));
    }
    let check = |target: Option<FolderId>| {
        if target == current_folder {
            rmac_ui::MenuCheck::On
        } else {
            rmac_ui::MenuCheck::None
        }
    };
    let mut move_to = rmac_ui::ContextMenu::new(pos).checked_item(
        "All Notes",
        check(None),
        Box::new(MoveNoteToFolderAction { folder_id: None }),
    );
    for folder in folders {
        let folder_id = folder.id;
        move_to = move_to.checked_item(
            folder.name.clone(),
            check(Some(folder_id)),
            Box::new(MoveNoteToFolderAction {
                folder_id: Some(folder_id),
            }),
        );
    }
    let m = rmac_ui::ContextMenu::new(pos)
        .item(
            if pinned { "Unpin Note" } else { "Pin Note" },
            Box::new(TogglePin),
        )
        .item(
            if locked { "Remove Lock" } else { "Lock Note" },
            Box::new(ToggleLockNote),
        )
        .separator()
        .submenu("Move to", move_to);
    // A locked note's title/body/tags are sealed, so it is never
    // duplicated into a plaintext note (see `duplicate_note`'s own guard);
    // the menu dims the item rather than offering a command that no-ops.
    let m = if locked {
        m.disabled_item("Duplicate", Box::new(DuplicateNote))
    } else {
        m.command_item(
            "Duplicate",
            rmac_ui::shortcuts::DUPLICATE,
            Box::new(DuplicateNote),
        )
    };
    m.danger_item("Delete", Box::new(DeleteSelectedNote))
        .separator()
        .item("Open Note in New Window", Box::new(OpenNoteInNewWindow))
}

fn build_folder_context_menu(
    pos: Point<Pixels>,
    sort_order: Option<SortOrder>,
) -> rmac_ui::ContextMenu {
    let check = |order| {
        if sort_order == Some(order) {
            rmac_ui::MenuCheck::On
        } else {
            rmac_ui::MenuCheck::None
        }
    };
    let sort_by = rmac_ui::ContextMenu::new(pos)
        .checked_item(
            "Date Edited",
            check(SortOrder::Edited),
            Box::new(SortByEdited),
        )
        .checked_item(
            "Date Created",
            check(SortOrder::Created),
            Box::new(SortByCreated),
        )
        .checked_item("Title", check(SortOrder::Title), Box::new(SortByTitle));
    rmac_ui::ContextMenu::new(pos)
        .item("New Folder", Box::new(CreateFolder))
        .item("Rename Folder…", Box::new(RenameSelectedFolder))
        .danger_item("Delete Folder…", Box::new(DeleteSelectedFolder))
        .separator()
        .submenu("Sort Notes By", sort_by)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpinned_unlocked_note_labels() {
        assert_eq!(
            note_context_menu_labels(false, false, false),
            vec![
                "Pin Note",
                "Lock Note",
                "Move to",
                "Duplicate",
                "Delete",
                "Open Note in New Window",
            ],
        );
    }

    #[test]
    fn pinned_locked_note_labels() {
        assert_eq!(
            note_context_menu_labels(true, true, false),
            vec![
                "Unpin Note",
                "Remove Lock",
                "Move to",
                "Duplicate",
                "Delete",
                "Open Note in New Window",
            ],
        );
    }

    #[test]
    fn deleted_note_shows_only_recovery_items() {
        // Even a pinned, locked note that is also deleted gets the Trash
        // menu: Recently Deleted overrides every other state, matching
        // `build_note_context_menu`'s early return.
        assert_eq!(
            note_context_menu_labels(true, true, true),
            vec!["Put Back", "Delete Immediately…"],
        );
    }

    #[test]
    fn folder_menu_labels_are_stable() {
        assert_eq!(
            folder_context_menu_labels(),
            vec![
                "New Folder",
                "Rename Folder…",
                "Delete Folder…",
                "Sort Notes By"
            ],
        );
    }
}
