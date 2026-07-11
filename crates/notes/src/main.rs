//! rmac Notes — an Apple Notes-style app, built for fidelity.
//!
//! Three columns: folders sidebar │ notes list │ editor. The editor splits each
//! note into a big bold title (first line) and a regular body, exactly like
//! macOS Notes. Notes are `.md` files under `~/Documents/rmac-notes`,
//! auto-saved on a 1.5s debounce.
//!
//! Real folders live as subdirectories of the notes dir. Notes carry per-note
//! tags (persisted as a trailing `<!--tags: ...-->` comment line) and the
//! editor has a format bar that inserts markdown blocks (headings, bullet
//! lists, checklists) and a live preview that renders them with macOS styling.

mod storage;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    actions, div, prelude::FluentBuilder as _, px, AnyElement, AppContext as _, Context, Div,
    Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _};
use rmac_editor::InputState;
use rmac_ui::{mac, Button, InputEvent, SearchField, TextField};

const FOLDERS_W: f32 = 200.0;
const LIST_W: f32 = 292.0;

actions!(
    notes,
    [
        NewNote,
        NewFolder,
        DeleteNote,
        TogglePreview,
        RenameFolder,
        DeleteFolder,
        TogglePin,
        SortByEdited,
        SortByCreated,
        SortByTitle
    ]
);

/// The order the note list is sorted in (matches macOS Notes' View ▸ Sort By).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Edited,
    Created,
    Title,
}

impl SortBy {
    fn label(self) -> &'static str {
        match self {
            SortBy::Edited => "Date Edited",
            SortBy::Created => "Date Created",
            SortBy::Title => "Title",
        }
    }
    fn id(self) -> &'static str {
        match self {
            SortBy::Edited => "edited",
            SortBy::Created => "created",
            SortBy::Title => "title",
        }
    }
    fn from_id(s: &str) -> Self {
        match s {
            "created" => SortBy::Created,
            "title" => SortBy::Title,
            _ => SortBy::Edited,
        }
    }
}

/// Which folder the user is browsing. `All` is the virtual "All Notes" view.
#[derive(Clone, PartialEq)]
enum FolderSel {
    All,
    Folder(String),
}

struct Folder {
    name: SharedString,
    count: usize,
}

struct Note {
    path: PathBuf,
    /// Parent folder name, or `None` for notes in the root (no folder).
    folder: Option<String>,
    title: SharedString,
    snippet: SharedString,
    date: SharedString,
    tags: Vec<String>,
    /// Raw modified / created times, for the Sort By order.
    mtime: SystemTime,
    ctime: SystemTime,
}

struct NotesView {
    dir: PathBuf,
    notes: Vec<Note>,
    folders: Vec<Folder>,
    folder_sel: FolderSel,
    /// Index into `self.notes` of the open note (stable across filtering).
    selected: Option<usize>,
    /// In-place folder rename: (original name, edit field).
    renaming_folder: Option<(String, Entity<InputState>)>,
    title: Entity<InputState>,
    body: Entity<InputState>,
    tags_input: Entity<InputState>,
    search: Entity<InputState>,
    preview: bool,
    last_saved: String,
    /// Paths of pinned notes (sort to the top), persisted to a `.pinned` file.
    pinned: HashSet<PathBuf>,
    /// Order the note list is sorted in, persisted to a `.sort` file.
    sort_by: SortBy,
    /// Open right-click menu: window-relative position + which menu.
    menu: Option<(Point<Pixels>, NoteMenuKind)>,
    /// Folder name awaiting a delete confirmation (shared alert), if any.
    confirm_delete_folder: Option<String>,
    storage_error: Option<SharedString>,
    focus: FocusHandle,
}

/// Which right-click menu is open in Notes.
#[derive(Clone, Copy)]
enum NoteMenuKind {
    Folder,
    Note,
}

impl NotesView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(home).join("Documents").join("rmac-notes");
        let mut storage_error = storage::create_dir(
            &storage::RealStorage,
            storage::Operation::CreateFolder,
            &dir,
        )
        .err()
        .map(|failure| failure.to_string().into());

        cx.bind_keys([
            KeyBinding::new("cmd-n", NewNote, Some("Notes")),
            KeyBinding::new("shift-cmd-n", NewFolder, Some("Notes")),
            KeyBinding::new("cmd-backspace", DeleteNote, Some("Notes")),
            KeyBinding::new("shift-cmd-p", TogglePreview, Some("Notes")),
        ]);

        // Title is single-line; body is multi-line soft-wrapped.
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let body = rmac_editor::multiline("Note", window, cx);
        let tags_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add tags, comma separated"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        // Re-render tag pills as the user edits the tags field.
        cx.observe(&tags_input, |_, _, cx| cx.notify()).detach();

        let pinned = load_pins(&dir).unwrap_or_else(|failure| {
            storage_error = Some(failure.to_string().into());
            HashSet::new()
        });
        let sort_by = load_sort(&dir).unwrap_or_else(|failure| {
            storage_error = Some(failure.to_string().into());
            SortBy::Edited
        });
        let mut view = Self {
            notes: Vec::new(),
            folders: Vec::new(),
            folder_sel: FolderSel::All,
            dir,
            selected: None,
            renaming_folder: None,
            title,
            body,
            tags_input,
            search,
            preview: false,
            last_saved: String::new(),
            pinned,
            sort_by,
            menu: None,
            confirm_delete_folder: None,
            storage_error,
            focus: cx.focus_handle(),
        };
        view.reload(None, cx);

        if !view.notes.is_empty() {
            view.select(0, window, cx);
        }

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let Some(this) = this.upgrade() else { break };
            if cx
                .update_entity(&this, |view: &mut NotesView, cx| view.save_current(cx))
                .is_err()
            {
                break;
            }
        })
        .detach();

        view
    }

    // ---- model ----

    /// Does a note belong to the folder currently being browsed?
    fn in_folder(&self, note: &Note) -> bool {
        match &self.folder_sel {
            FolderSel::All => true,
            FolderSel::Folder(name) => note.folder.as_deref() == Some(name.as_str()),
        }
    }

    /// Rescan folders and notes from disk, preserving the open note by path.
    /// Pin or unpin the selected note (sorts it to the top), persisting the set.
    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self
            .selected
            .and_then(|i| self.notes.get(i))
            .map(|n| n.path.clone())
        else {
            return;
        };
        let mut next = self.pinned.clone();
        if !next.remove(&path) {
            next.insert(path.clone());
        }
        match save_pins(&self.dir, &next) {
            Ok(()) => {
                self.pinned = next;
                self.storage_error = None;
                self.reload(Some(path), cx);
            }
            Err(failure) => self.record_storage_failure(failure, cx),
        }
    }

    /// Change the note-list sort order, persist it, and re-sort the list.
    fn set_sort(&mut self, sort: SortBy, cx: &mut Context<Self>) {
        if self.sort_by == sort {
            return;
        }
        match save_sort(&self.dir, sort) {
            Ok(()) => {
                self.sort_by = sort;
                self.storage_error = None;
                self.reload(None, cx);
            }
            Err(failure) => self.record_storage_failure(failure, cx),
        }
    }

    fn reload(&mut self, preserve: Option<PathBuf>, cx: &mut Context<Self>) {
        let keep = preserve.or_else(|| {
            self.selected
                .and_then(|i| self.notes.get(i))
                .map(|n| n.path.clone())
        });

        let mut notes = scan_notes(&self.dir);
        // Apply the chosen sort order, then sort pinned notes first (stable, so
        // the chosen order is preserved within each group).
        match self.sort_by {
            SortBy::Edited => notes.sort_by(|a, b| b.mtime.cmp(&a.mtime)),
            SortBy::Created => notes.sort_by(|a, b| b.ctime.cmp(&a.ctime)),
            SortBy::Title => {
                notes.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            }
        }
        self.pinned.retain(|p| p.exists());
        notes.sort_by_key(|n| !self.pinned.contains(&n.path));
        self.notes = notes;
        let names = scan_folders(&self.dir);
        self.folders = names
            .into_iter()
            .map(|name| {
                let count = self
                    .notes
                    .iter()
                    .filter(|n| n.folder.as_deref() == Some(name.as_str()))
                    .count();
                Folder {
                    name: name.into(),
                    count,
                }
            })
            .collect();

        self.selected = keep.and_then(|p| self.notes.iter().position(|n| n.path == p));
        cx.notify();
    }

    /// The full document text (title line + body + tags) for the open note.
    fn doc(&self, cx: &Context<Self>) -> String {
        let t = self.title.read(cx).value().to_string();
        let b = self.body.read(cx).value().to_string();
        let tags = parse_tags(&self.tags_input.read(cx).value());
        let mut s = if t.is_empty() && b.is_empty() {
            String::new()
        } else {
            format!("{t}\n{b}")
        };
        if !tags.is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&format!("<!--tags: {}-->", tags.join(", ")));
        }
        s
    }

    /// Split a stored document into the title, body, and tags fields.
    fn load_doc(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (t, b, tags) = parse_doc(text);
        self.title.update(cx, |s, cx| s.set_value(t, window, cx));
        self.body.update(cx, |s, cx| s.set_value(b, window, cx));
        self.tags_input
            .update(cx, |s, cx| s.set_value(tags.join(", "), window, cx));
    }

    fn record_storage_failure(&mut self, failure: storage::Failure, cx: &mut Context<Self>) {
        self.storage_error = Some(failure.to_string().into());
        cx.notify();
    }

    fn save_current(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.selected else { return true };
        let doc = self.doc(cx);
        if doc == self.last_saved {
            return true;
        }
        // Compute every derived value before taking a mutable borrow of `notes`.
        let t = self.title.read(cx).value().to_string();
        let b = self.body.read(cx).value().to_string();
        let meta = format!("{t}\n{b}");
        let tags = parse_tags(&self.tags_input.read(cx).value());
        let Some(path) = self.notes.get(ix).map(|note| note.path.clone()) else {
            return true;
        };
        if let Err(failure) = storage::write(
            &storage::RealStorage,
            storage::Operation::SaveNote,
            &path,
            &doc,
        ) {
            self.record_storage_failure(failure, cx);
            return false;
        }
        if let Some(note) = self.notes.get_mut(ix) {
            note.title = title_of(&meta).into();
            note.snippet = snippet_of(&meta).into();
            note.date = date_label(SystemTime::now()).into();
            note.tags = tags;
            self.last_saved = doc;
            self.storage_error = None;
            cx.notify();
        }
        true
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_current(cx) {
            return;
        }
        let Some(path) = self.notes.get(ix).map(|note| note.path.clone()) else {
            return;
        };
        let text = match storage::read(&storage::RealStorage, storage::Operation::LoadNote, &path) {
            Ok(text) => text,
            Err(failure) => {
                self.record_storage_failure(failure, cx);
                return;
            }
        };
        self.load_doc(&text, window, cx);
        self.selected = Some(ix);
        self.last_saved = self.doc(cx);
        cx.notify();
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_current(cx) {
            return;
        }
        let target = match &self.folder_sel {
            FolderSel::Folder(n) => self.dir.join(n),
            FolderSel::All => self.dir.clone(),
        };
        if let Err(failure) = storage::create_dir(
            &storage::RealStorage,
            storage::Operation::CreateFolder,
            &target,
        ) {
            self.record_storage_failure(failure, cx);
            return;
        }
        let path = unique_path(&target);
        if let Err(failure) = storage::write(
            &storage::RealStorage,
            storage::Operation::CreateNote,
            &path,
            "",
        ) {
            self.record_storage_failure(failure, cx);
            return;
        }
        self.storage_error = None;
        self.reload(Some(path.clone()), cx);
        if let Some(ix) = self.notes.iter().position(|n| n.path == path) {
            self.select(ix, window, cx);
            let handle = self.title.read(cx).focus_handle(cx);
            window.focus(&handle);
        }
    }

    fn delete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        if ix >= self.notes.len() {
            return;
        }
        let path = self.notes[ix].path.clone();
        if let Err(failure) =
            storage::remove_file(&storage::RealStorage, storage::Operation::DeleteNote, &path)
        {
            self.record_storage_failure(failure, cx);
            return;
        }
        self.storage_error = None;
        if self.pinned.remove(&path) {
            if let Err(failure) = save_pins(&self.dir, &self.pinned) {
                self.record_storage_failure(failure, cx);
            }
        }
        self.selected = None;
        self.last_saved.clear();
        self.reload(None, cx);

        // Re-open the next visible note in this folder, if any.
        if let Some(ix) = self.notes.iter().position(|n| self.in_folder(n)) {
            self.select(ix, window, cx);
        } else {
            self.load_doc("", window, cx);
        }
        cx.notify();
    }

    // ---- folders ----

    fn select_folder(&mut self, sel: FolderSel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_current(cx) {
            return;
        }
        self.folder_sel = sel;
        self.renaming_folder = None;
        match self.notes.iter().position(|n| self.in_folder(n)) {
            Some(ix) => self.select(ix, window, cx),
            None => {
                self.selected = None;
                self.last_saved.clear();
                self.load_doc("", window, cx);
                cx.notify();
            }
        }
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = unique_folder(&self.dir);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("New Folder")
            .to_string();
        if let Err(failure) = storage::create_dir(
            &storage::RealStorage,
            storage::Operation::CreateFolder,
            &path,
        ) {
            self.record_storage_failure(failure, cx);
            return;
        }
        self.storage_error = None;
        self.reload(None, cx);
        self.folder_sel = FolderSel::Folder(name.clone());
        self.rename_folder_start(window, cx);
    }

    fn rename_folder_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let FolderSel::Folder(name) = self.folder_sel.clone() else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        // Commit the typed name on Enter, or when focus leaves the field
        // ("type name, click away"). Escape cancels via on_key_down before
        // any blur fires, so a cancel never reaches this commit path.
        cx.subscribe(&input, |this, _input, ev: &InputEvent, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.rename_folder_commit(cx),
            _ => {}
        })
        .detach();
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle);
        self.renaming_folder = Some((name, input));
        cx.notify();
    }

    fn rename_folder_commit(&mut self, cx: &mut Context<Self>) {
        let Some((old, input)) = self.renaming_folder.take() else {
            return;
        };
        let new_name = input.read(cx).value().trim().to_string();
        let mut preserve = self
            .selected
            .and_then(|i| self.notes.get(i))
            .map(|n| n.path.clone());
        if !new_name.is_empty() && new_name != old {
            let src = self.dir.join(&old);
            let dst = self.dir.join(&new_name);
            if !dst.exists() {
                if let Err(failure) = storage::rename(
                    &storage::RealStorage,
                    storage::Operation::RenameFolder,
                    &src,
                    &dst,
                ) {
                    self.record_storage_failure(failure, cx);
                    self.reload(preserve, cx);
                    return;
                }
                self.storage_error = None;
                // The open note moved with its folder — remap its path.
                if let Some(p) = preserve.clone() {
                    if let Ok(rel) = p.strip_prefix(&src) {
                        preserve = Some(dst.join(rel));
                    }
                }
                let mut pins_changed = false;
                self.pinned = self
                    .pinned
                    .iter()
                    .map(|path| {
                        if let Ok(relative) = path.strip_prefix(&src) {
                            pins_changed = true;
                            dst.join(relative)
                        } else {
                            path.clone()
                        }
                    })
                    .collect();
                if pins_changed {
                    if let Err(failure) = save_pins(&self.dir, &self.pinned) {
                        self.record_storage_failure(failure, cx);
                    }
                }
                if let FolderSel::Folder(n) = &self.folder_sel {
                    if n == &old {
                        self.folder_sel = FolderSel::Folder(new_name.clone());
                    }
                }
            } else {
                self.record_storage_failure(
                    storage::Failure::message(
                        storage::Operation::RenameFolder,
                        &src,
                        "a folder with that name already exists",
                    ),
                    cx,
                );
            }
        }
        self.reload(preserve, cx);
    }

    /// Escape path: discard the rename and drop the edit field without renaming.
    fn rename_folder_cancel(&mut self, cx: &mut Context<Self>) {
        if self.renaming_folder.take().is_some() {
            cx.notify();
        }
    }

    fn delete_folder(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let FolderSel::Folder(name) = self.folder_sel.clone() else {
            return;
        };
        // Persist any pending edits in the open note before touching the disk —
        // the note may live inside the folder we're about to remove.
        if !self.save_current(cx) {
            return;
        }
        self.confirm_delete_folder = Some(name);
        cx.notify();
    }

    /// Confirm button of the delete-folder alert.
    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(name) = self.confirm_delete_folder.take() {
            self.delete_folder_confirmed(&name, cx);
        }
        cx.notify();
    }

    fn delete_folder_confirmed(&mut self, name: &str, cx: &mut Context<Self>) {
        let path = self.dir.join(name);
        if let Err(failure) = storage::remove_dir_all(
            &storage::RealStorage,
            storage::Operation::DeleteFolder,
            &path,
        ) {
            self.record_storage_failure(failure, cx);
            return;
        }
        self.storage_error = None;
        let previous_pin_count = self.pinned.len();
        self.pinned.retain(|pinned| !pinned.starts_with(&path));
        if self.pinned.len() != previous_pin_count {
            if let Err(failure) = save_pins(&self.dir, &self.pinned) {
                self.record_storage_failure(failure, cx);
            }
        }
        if self.folder_sel == FolderSel::Folder(name.to_string()) {
            self.folder_sel = FolderSel::All;
        }
        self.selected = None;
        self.last_saved.clear();
        self.reload(None, cx);
    }

    // ---- format blocks ----

    /// Attach an image: pick a file, copy it next to the note, insert a markdown ref.
    fn attach_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let dir = self.dir.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(src) = paths.into_iter().next() else {
                return;
            };
            let requested_name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".to_string());
            let dst = unique_named_path(&dir, &requested_name);
            let name = dst
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(requested_name);
            let result = storage::copy(
                &storage::RealStorage,
                storage::Operation::Attach,
                &src,
                &dst,
            );
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    this.storage_error = None;
                    this.insert_token(&format!("\n![]({name})\n"), window, cx);
                }
                Err(failure) => this.record_storage_failure(failure, cx),
            });
        })
        .detach();
    }

    fn insert_token(&mut self, tok: &str, window: &mut Window, cx: &mut Context<Self>) {
        let tok = tok.to_string();
        self.body.update(cx, |s, cx| s.insert(tok, window, cx));
        let handle = self.body.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    /// Toggle the checkbox state of the markdown checklist on `line_ix`.
    fn toggle_check(&mut self, line_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.body.read(cx).value().to_string();
        let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
        if let Some(l) = lines.get_mut(line_ix) {
            let indent: String = l.chars().take_while(|c| c.is_whitespace()).collect();
            let trimmed = l.trim_start();
            if let Some(rest) = trimmed.strip_prefix("- [ ]") {
                *l = format!("{indent}- [x]{rest}");
            } else if let Some(rest) = trimmed.strip_prefix("- [x]") {
                *l = format!("{indent}- [ ]{rest}");
            }
        }
        let new = lines.join("\n");
        self.body.update(cx, |s, cx| s.set_value(new, window, cx));
        cx.notify();
    }

    // ---- chrome ----

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .child(div().w(px(FOLDERS_W - 80.0)))
            .child(
                div()
                    .w(px(LIST_W))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_3()
                    .child(
                        Button::new("sort", "")
                            .icon(IconName::SortDescending)
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("Sort By")
                            .dropdown_menu({
                                let current = self.sort_by;
                                move |menu, _, _| {
                                    menu.menu_with_check(
                                        SortBy::Edited.label(),
                                        current == SortBy::Edited,
                                        Box::new(SortByEdited),
                                    )
                                    .menu_with_check(
                                        SortBy::Created.label(),
                                        current == SortBy::Created,
                                        Box::new(SortByCreated),
                                    )
                                    .menu_with_check(
                                        SortBy::Title.label(),
                                        current == SortBy::Title,
                                        Box::new(SortByTitle),
                                    )
                                }
                            }),
                    )
                    .child(
                        Button::new("compose", "")
                            .icon(IconName::Plus)
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("New Note")
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_4()
                    .child(
                        Button::new("delete", "")
                            .icon(IconName::Delete)
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("Delete Note")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.delete_current(window, cx)),
                            ),
                    ),
            );
        rmac_ui::toolbar(row)
    }

    fn render_folders(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let all_count = self.notes.len();
        let all_selected = self.folder_sel == FolderSel::All;

        // "All Notes" virtual row.
        let all_row = div()
            .id("folder-all")
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(px(6.0))
            .when(all_selected, |el: Stateful<Div>| {
                el.bg(mac::sidebar_selection())
            })
            .when(!all_selected, |el: Stateful<Div>| {
                el.hover(|h| h.bg(mac::hover()))
            })
            .child(
                Icon::new(IconName::Folder)
                    .text_color(mac::notes_accent())
                    .with_size(Size::Small),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(mac::text())
                    .child("All Notes"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(mac::text_tertiary())
                    .child(all_count.to_string()),
            )
            .on_click(
                cx.listener(|this, _, window, cx| this.select_folder(FolderSel::All, window, cx)),
            );

        let mut col = div()
            .w(px(FOLDERS_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_3()
            .px_2()
            .bg(mac::sidebar())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pb_1()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .child("ON MY MAC"),
                    )
                    .child(
                        Button::new("new-folder", "")
                            .icon(IconName::Plus)
                            .ghost()
                            .with_size(Size::XSmall)
                            .tooltip("New Folder")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_folder(window, cx)),
                            ),
                    ),
            )
            .child(all_row);

        for (fidx, folder) in self.folders.iter().enumerate() {
            let name = folder.name.to_string();
            let selected = self.folder_sel == FolderSel::Folder(name.clone());

            // In-place rename field for this folder.
            if let Some((renaming, input)) = &self.renaming_folder {
                if renaming == &name {
                    col = col.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            // Escape cancels the rename (keeps the original name);
                            // blur/Enter commit it.
                            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                if ev.keystroke.key == "escape" {
                                    this.rename_folder_cancel(cx);
                                }
                            }))
                            .child(
                                Icon::new(IconName::Folder)
                                    .text_color(mac::notes_accent())
                                    .with_size(Size::Small),
                            )
                            .child(div().flex_1().child(TextField::new(input).small())),
                    );
                    continue;
                }
            }

            let sel_click = name.clone();
            let sel_menu = name.clone();
            let row = div()
                .id(("folder", fidx))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1p5()
                .rounded(px(6.0))
                .when(selected, |el: Stateful<Div>| {
                    el.bg(mac::sidebar_selection())
                })
                .when(!selected, |el: Stateful<Div>| {
                    el.hover(|h| h.bg(mac::hover()))
                })
                .child(
                    Icon::new(IconName::Folder)
                        .text_color(mac::notes_accent())
                        .with_size(Size::Small),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(mac::text())
                        .truncate()
                        .child(folder.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(mac::text_tertiary())
                        .child(folder.count.to_string()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_folder(FolderSel::Folder(sel_click.clone()), window, cx)
                }))
                // Right-click selects this folder so the context-menu actions target it.
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        this.select_folder(FolderSel::Folder(sel_menu.clone()), window, cx);
                        this.menu = Some((ev.position, NoteMenuKind::Folder));
                        cx.notify();
                    }),
                );
            col = col.child(row);
        }

        col
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<AnyElement> = Vec::new();
        let visible: Vec<usize> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| self.in_folder(n))
            .filter(|(_, n)| {
                q.is_empty()
                    || n.title.to_lowercase().contains(&q)
                    || n.snippet.to_lowercase().contains(&q)
                    || n.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .map(|(ix, _)| ix)
            .collect();

        let any_pinned = visible
            .iter()
            .any(|&ix| self.pinned.contains(&self.notes[ix].path));
        let mut header_pinned = false;
        let mut header_notes = false;

        let last = visible.len().saturating_sub(1);
        for (pos, &ix) in visible.iter().enumerate() {
            let note = &self.notes[ix];
            let selected = self.selected == Some(ix);
            let is_pinned = self.pinned.contains(&note.path);
            let tags = note.tags.clone();

            // Section headers, macOS-style: "Pinned" then "Notes".
            if any_pinned {
                if is_pinned && !header_pinned {
                    header_pinned = true;
                    items.push(list_section_header("Pinned"));
                } else if !is_pinned && !header_notes {
                    header_notes = true;
                    items.push(list_section_header("Notes"));
                }
            }

            items.push(
                div()
                    .id(("note", ix))
                    .mx_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.0))
                    .when(selected, |el: Stateful<Div>| el.bg(mac::notes_selection()))
                    .when(!selected, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(mac::hover()))
                    })
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .when(is_pinned, |el| {
                                        el.child(
                                            Icon::new(IconName::Star)
                                                .text_color(mac::notes_accent())
                                                .with_size(Size::XSmall),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(14.0))
                                            .font_weight(mac::SEMIBOLD)
                                            .text_color(mac::text())
                                            .truncate()
                                            .child(note.title.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(mac::MEDIUM)
                                            .text_color(mac::text())
                                            .child(note.date.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.0))
                                            .text_color(mac::text_secondary())
                                            .truncate()
                                            .child(note.snippet.clone()),
                                    ),
                            )
                            .when(!tags.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_1()
                                        .pt_0p5()
                                        .children(tags.into_iter().map(tag_pill)),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(ix, window, cx)))
                    // Right-click selects this note so the menu acts on it.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            this.select(ix, window, cx);
                            this.menu = Some((ev.position, NoteMenuKind::Note));
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
            if pos != last {
                let next_selected = visible
                    .get(pos + 1)
                    .map(|&n| self.selected == Some(n))
                    .unwrap_or(false);
                if !selected && !next_selected {
                    items.push(
                        div()
                            .mx_4()
                            .h(px(1.0))
                            .bg(mac::separator())
                            .into_any_element(),
                    );
                }
            }
        }

        div()
            .w(px(LIST_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .bg(mac::list())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div().px_2().py_2().child(
                    div()
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .px_2()
                        .rounded(px(7.0))
                        .bg(mac::control_fill())
                        .child(
                            div()
                                .flex_1()
                                .child(SearchField::new(&self.search).appearance(false)),
                        ),
                ),
            )
            .child(
                div()
                    .id("notes-scroll")
                    .flex_1()
                    .py_1()
                    .overflow_y_scroll()
                    .child(div().v_flex().children(items)),
            )
    }

    /// Footer with live word + character counts for the active note body.
    fn render_count_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let body = self.body.read(cx).value().to_string();
        let words = body.split_whitespace().count();
        let chars = body.chars().count();
        div()
            .flex_none()
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .border_t_1()
            .border_color(mac::separator())
            .bg(mac::window())
            .text_size(px(11.0))
            .text_color(mac::text_tertiary())
            .child(format!("{words} word{}", if words == 1 { "" } else { "s" }))
            .child(div().text_color(mac::text_tertiary()).child("•"))
            .child(format!(
                "{chars} character{}",
                if chars == 1 { "" } else { "s" }
            ))
    }

    fn render_format_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let preview = self.preview;
        let btn = |id: &'static str,
                   label: &'static str,
                   tip: &'static str,
                   tok: &'static str,
                   cx: &mut Context<Self>| {
            Button::new(id, label)
                .ghost()
                .with_size(Size::Small)
                .disabled(preview)
                .tooltip(tip)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.insert_token(tok, window, cx)),
                )
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .px(px(40.0))
            .py_1()
            .border_b_1()
            .border_color(mac::separator())
            .child(btn("fmt-h1", "Title", "Heading", "# ", cx))
            .child(btn("fmt-h2", "Heading", "Subheading", "## ", cx))
            .child(btn("fmt-bullet", "• List", "Bulleted List", "- ", cx))
            .child(btn("fmt-check", "☑ Checklist", "Checklist", "- [ ] ", cx))
            .child(
                Button::new("fmt-attach", "📎 Attach")
                    .ghost()
                    .with_size(Size::Small)
                    .disabled(preview)
                    .tooltip("Attach Image")
                    .on_click(cx.listener(|this, _, window, cx| this.attach_image(window, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("preview", if preview { "Edit" } else { "Preview" })
                    .icon(if preview {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    })
                    .ghost()
                    .with_size(Size::Small)
                    .when(preview, |b| b.primary())
                    .tooltip("Toggle Preview")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.preview = !this.preview;
                        cx.notify();
                    })),
            )
    }

    /// Render the note body as styled markdown blocks (headings, bullets,
    /// checklists). Checklist boxes are clickable.
    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body.read(cx).value().to_string();
        let mut blocks: Vec<AnyElement> = Vec::new();
        for (i, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            let el: AnyElement = if let Some(path) = parse_image(trimmed) {
                gpui::img(self.dir.join(&path))
                    .max_w(px(380.0))
                    .rounded(px(6.0))
                    .into_any_element()
            } else if let Some(rest) = trimmed
                .strip_prefix("- [ ]")
                .or_else(|| trimmed.strip_prefix("- [x]"))
            {
                let checked = trimmed.starts_with("- [x]");
                checklist_row(i, checked, rest.trim(), cx)
            } else if let Some(rest) = trimmed.strip_prefix("## ") {
                div()
                    .pt_2()
                    .text_size(px(20.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(rest.to_string())
                    .into_any_element()
            } else if let Some(rest) = trimmed.strip_prefix("# ") {
                div()
                    .pt_2()
                    .text_size(px(26.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(rest.to_string())
                    .into_any_element()
            } else if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .w(px(16.0))
                            .text_color(mac::text_secondary())
                            .child("•"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.0))
                            .text_color(mac::text())
                            .child(rest.to_string()),
                    )
                    .into_any_element()
            } else if trimmed.is_empty() {
                div().h(px(10.0)).into_any_element()
            } else {
                div()
                    .text_size(px(16.0))
                    .line_height(px(24.0))
                    .text_color(mac::text())
                    .child(line.to_string())
                    .into_any_element()
            };
            blocks.push(el);
        }

        div()
            .id("preview-scroll")
            .flex_1()
            .px(px(44.0))
            .pt_2()
            .pb_4()
            .overflow_y_scroll()
            .child(div().v_flex().gap_1().children(blocks))
    }

    fn render_tags_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let tags = parse_tags(&self.tags_input.read(cx).value());
        div()
            .px(px(40.0))
            .py_1()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_1()
            .child(
                Icon::new(IconName::Folder)
                    .text_color(mac::text_tertiary())
                    .with_size(Size::XSmall),
            )
            .children(tags.into_iter().map(tag_pill))
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.0))
                    .child(TextField::new(&self.tags_input).appearance(false).small()),
            )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.selected.is_some() {
            let meta = self
                .selected
                .and_then(|ix| self.notes.get(ix))
                .map(|n| n.date.clone())
                .unwrap_or_default();
            div()
                .size_full()
                .v_flex()
                .bg(mac::window())
                .child(self.render_format_bar(cx))
                .child(
                    div()
                        .pt_3()
                        .pb_1()
                        .flex()
                        .justify_center()
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(meta),
                )
                // Big bold title (cascades into the single-line Input).
                .child(
                    div()
                        .px(px(44.0))
                        .pt_1()
                        .text_size(px(28.0))
                        .line_height(px(34.0))
                        .font_weight(mac::BOLD)
                        .text_color(mac::text())
                        .child(TextField::new(&self.title).appearance(false)),
                )
                .child(self.render_tags_bar(cx))
                // Body — editor or rendered preview.
                .child(if self.preview {
                    self.render_preview(cx).into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .px(px(44.0))
                        .pt_2()
                        .pb_4()
                        .text_size(px(16.0))
                        .line_height(px(24.0))
                        .text_color(mac::text())
                        .child(TextField::new(&self.body).h_full().appearance(false))
                        .into_any_element()
                })
                .child(self.render_count_footer(cx))
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(mac::window())
                .text_size(px(15.0))
                .text_color(mac::text_tertiary())
                .child("No Note Selected")
                .into_any_element()
        }
    }
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let storage_error = self.storage_error.clone();
        div()
            .track_focus(&self.focus)
            .key_context("Notes")
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_folder(window, cx)))
            .on_action(
                cx.listener(|this, _: &DeleteNote, window, cx| this.delete_current(window, cx)),
            )
            .on_action(cx.listener(|this, _: &TogglePreview, _, cx| {
                this.preview = !this.preview;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin(cx)))
            .on_action(
                cx.listener(|this, _: &SortByEdited, _, cx| this.set_sort(SortBy::Edited, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SortByCreated, _, cx| this.set_sort(SortBy::Created, cx)),
            )
            .on_action(cx.listener(|this, _: &SortByTitle, _, cx| this.set_sort(SortBy::Title, cx)))
            .on_action(cx.listener(|this, _: &RenameFolder, window, cx| {
                this.rename_folder_start(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DeleteFolder, window, cx| this.delete_folder(window, cx)),
            )
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                if this.save_current(cx) {
                    window.remove_window();
                }
            }))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(self.render_toolbar(cx))
            .when_some(storage_error, |el, message| {
                el.child(
                    div()
                        .id("storage-error")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::error_background())
                        .border_b_1()
                        .border_color(mac::error_border())
                        .text_size(px(12.0))
                        .text_color(mac::danger())
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.storage_error = None;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(self.render_folders(cx))
                    .child(self.render_list(cx))
                    .child(div().flex_1().child(self.render_editor(cx))),
            )
            .when_some(self.menu, |el: Div, (pos, kind)| {
                let menu = match kind {
                    NoteMenuKind::Folder => rmac_ui::ContextMenu::new(pos)
                        .item("Rename Folder", Box::new(RenameFolder))
                        .separator()
                        .danger_item("Delete Folder", Box::new(DeleteFolder)),
                    NoteMenuKind::Note => {
                        let pinned = self
                            .selected
                            .and_then(|i| self.notes.get(i))
                            .map(|n| self.pinned.contains(&n.path))
                            .unwrap_or(false);
                        rmac_ui::ContextMenu::new(pos)
                            .item(
                                if pinned { "Unpin Note" } else { "Pin Note" },
                                Box::new(TogglePin),
                            )
                            .separator()
                            .danger_item("Delete Note", Box::new(DeleteNote))
                    }
                };
                el.child(menu.render())
            })
            .when_some(self.confirm_delete_folder.clone(), |el: Div, name| {
                use rmac_ui::DialogButtonKind::{Destructive, Normal};
                el.child(rmac_ui::alert(
                    format!("Delete the folder \u{201c}{name}\u{201d}?"),
                    "All notes in this folder will be permanently deleted. This cannot be undone."
                        .to_string(),
                    vec![
                        rmac_ui::dialog_button("del-cancel", "Cancel", Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_delete_folder = None;
                                cx.notify();
                            }))
                            .into_any_element(),
                        rmac_ui::dialog_button("del-confirm", "Delete", Destructive)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx)))
                            .into_any_element(),
                    ],
                ))
            })
    }
}

// ---- small view helpers ----

/// A pill for a single tag.
fn tag_pill(tag: String) -> impl IntoElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(px(5.0))
        .bg(mac::notes_selection())
        .text_size(px(11.0))
        .font_weight(mac::MEDIUM)
        .text_color(mac::text())
        .child(format!("#{tag}"))
}

/// A checklist row in the preview, with a clickable box.
fn checklist_row(
    line_ix: usize,
    checked: bool,
    text: &str,
    cx: &mut Context<NotesView>,
) -> AnyElement {
    let box_el = div()
        .id(("check", line_ix))
        .w(px(18.0))
        .h(px(18.0))
        .mt(px(2.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(if checked {
            mac::notes_accent()
        } else {
            mac::text_tertiary()
        })
        .when(checked, |el: Stateful<Div>| el.bg(mac::notes_accent()))
        .when(checked, |el: Stateful<Div>| {
            el.child(
                Icon::new(IconName::Check)
                    .text_color(mac::text())
                    .with_size(Size::XSmall),
            )
        })
        .on_click(cx.listener(move |this, _, window, cx| this.toggle_check(line_ix, window, cx)));

    div()
        .flex()
        .items_start()
        .gap_2()
        .child(box_el)
        .child(
            div()
                .flex_1()
                .text_size(px(16.0))
                .text_color(if checked {
                    mac::text_secondary()
                } else {
                    mac::text()
                })
                .child(text.to_string()),
        )
        .into_any_element()
}

// ---- pure helpers ----

/// Parse a markdown image line `![alt](path)` → the path.
fn parse_image(line: &str) -> Option<String> {
    let l = line.trim();
    if !l.starts_with("![") {
        return None;
    }
    let open = l.find("](")?;
    let end = l.rfind(')')?;
    if end > open + 2 {
        Some(l[open + 2..end].to_string())
    } else {
        None
    }
}

fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().trim_start_matches('#').trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Split a stored document into (title, body, tags). The tags line is a
/// trailing `<!--tags: a, b-->` comment and is removed from the body.
fn parse_doc(text: &str) -> (String, String, Vec<String>) {
    let mut tags = Vec::new();
    let mut body_lines: Vec<&str> = text.lines().collect();
    // Tags live on the FINAL line only (trailing metadata). A `<!--tags: ...-->`
    // comment anywhere else in the body is real content and must be preserved.
    if let Some(last) = body_lines.last() {
        if let Some(rest) = last
            .trim()
            .strip_prefix("<!--tags:")
            .and_then(|r| r.strip_suffix("-->"))
        {
            tags = parse_tags(rest);
            body_lines.pop();
        }
    }
    let joined = body_lines.join("\n");
    let mut parts = joined.splitn(2, '\n');
    let title = parts.next().unwrap_or("").to_string();
    let body = parts.next().unwrap_or("").to_string();
    (title, body, tags)
}

fn title_of(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            let t: String = l.trim_start_matches('#').trim().chars().take(60).collect();
            if t.is_empty() {
                "New Note".to_string()
            } else {
                t
            }
        })
        .unwrap_or_else(|| "New Note".to_string())
}

fn snippet_of(body: &str) -> String {
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let _title = lines.next();
    let rest: String = lines.collect::<Vec<_>>().join(" ");
    if rest.is_empty() {
        "No additional text".to_string()
    } else {
        rest.chars().take(80).collect()
    }
}

/// macOS-style relative date: time today, "Yesterday", weekday this week, else M/D/YY.
fn date_label(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(dt.date_naive())
        .num_days();
    if days == 0 {
        let h = dt.hour();
        let (h12, ap) = if h == 0 {
            (12, "AM")
        } else if h < 12 {
            (h, "AM")
        } else if h == 12 {
            (12, "PM")
        } else {
            (h - 12, "PM")
        };
        format!("{}:{:02} {}", h12, dt.minute(), ap)
    } else if days == 1 {
        "Yesterday".to_string()
    } else if days < 7 {
        dt.format("%A").to_string()
    } else {
        format!("{}/{}/{:02}", dt.month(), dt.day(), dt.year() % 100)
    }
}

fn scan_folders(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_dir() {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    v.sort();
    v
}

fn collect_notes(dir: &Path, folder: Option<String>, out: &mut Vec<(Note, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let md = e.metadata().ok();
        let mtime = md
            .as_ref()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let ctime = md.as_ref().and_then(|m| m.created().ok()).unwrap_or(mtime);
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let (t, b, tags) = parse_doc(&raw);
        let meta = format!("{t}\n{b}");
        out.push((
            Note {
                title: title_of(&meta).into(),
                snippet: snippet_of(&meta).into(),
                date: date_label(mtime).into(),
                tags,
                folder: folder.clone(),
                path,
                mtime,
                ctime,
            },
            mtime,
        ));
    }
}

/// A small all-caps section header for the note list ("Pinned" / "Notes").
fn list_section_header(title: &'static str) -> AnyElement {
    div()
        .px_4()
        .pt_2()
        .pb_1()
        .text_size(px(11.0))
        .font_weight(mac::SEMIBOLD)
        .text_color(mac::text_tertiary())
        .child(title)
        .into_any_element()
}

/// Load the set of pinned note paths from `<dir>/.pinned` (one path per line).
fn load_pins(dir: &Path) -> Result<HashSet<PathBuf>, storage::Failure> {
    let path = dir.join(".pinned");
    match storage::read(&storage::RealStorage, storage::Operation::LoadPins, &path) {
        Ok(contents) => Ok({
            let s = contents;
            s.lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        }),
        Err(failure) if failure.error_kind == std::io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(failure) => Err(failure),
    }
}

fn save_pins(dir: &Path, pins: &HashSet<PathBuf>) -> Result<(), storage::Failure> {
    let body = pins
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    storage::write(
        &storage::RealStorage,
        storage::Operation::SavePins,
        &dir.join(".pinned"),
        body,
    )
}

/// Load the persisted sort order from `<dir>/.sort` (defaults to Date Edited).
fn load_sort(dir: &Path) -> Result<SortBy, storage::Failure> {
    let path = dir.join(".sort");
    match storage::read(&storage::RealStorage, storage::Operation::LoadSort, &path) {
        Ok(contents) => Ok(SortBy::from_id(contents.trim())),
        Err(failure) if failure.error_kind == std::io::ErrorKind::NotFound => Ok(SortBy::Edited),
        Err(failure) => Err(failure),
    }
}

fn save_sort(dir: &Path, sort: SortBy) -> Result<(), storage::Failure> {
    storage::write(
        &storage::RealStorage,
        storage::Operation::SaveSort,
        &dir.join(".sort"),
        sort.id(),
    )
}

fn scan_notes(dir: &Path) -> Vec<Note> {
    let mut entries: Vec<(Note, SystemTime)> = Vec::new();
    collect_notes(dir, None, &mut entries);
    for name in scan_folders(dir) {
        collect_notes(&dir.join(&name), Some(name), &mut entries);
    }
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.into_iter().map(|(n, _)| n).collect()
}

fn unique_path(dir: &Path) -> PathBuf {
    let mut n = 1;
    loop {
        let path = dir.join(format!("note-{n}.md"));
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

fn unique_named_path(dir: &Path, name: &str) -> PathBuf {
    let requested = dir.join(name);
    if !requested.exists() {
        return requested;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("attachment");
    let extension = path.extension().and_then(|extension| extension.to_str());
    for suffix in 2..10_000 {
        let candidate = match extension {
            Some(extension) => dir.join(format!("{stem} {suffix}.{extension}")),
            None => dir.join(format!("{stem} {suffix}")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    requested
}

fn unique_folder(dir: &Path) -> PathBuf {
    let mut n = 0;
    loop {
        let name = if n == 0 {
            "New Folder".to_string()
        } else {
            format!("New Folder {n}")
        };
        let path = dir.join(&name);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

fn main() {
    rmac_ui::boot("Notes", 1080.0, 720.0, |window, cx| {
        NotesView::new(window, cx)
    });
}
