//! Text Editor startup, document-window, and interaction controller.

mod alerts;
mod conflicts;
mod document_io;
mod document_state;
mod editing;
mod file_ops;
mod format_extras;
mod format_text;
mod lifecycle;
mod long_line_view;
mod opening;
mod printing;
mod recovery_state;
mod render;
mod responsive_layout;
mod saving;
mod startup;
mod text_assist;

use std::{
    ffi::OsString,
    path::Path,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, KeyBinding, PathPromptOptions,
    SharedString, Subscription, Window,
};
use gpui_component::Root;
use notify::Watcher as _;
use rmac_editor::rich;
use rmac_ui::{InputEvent, InputState, Position, Rope, RopeExt as _};

#[cfg(target_os = "linux")]
use crate::PrintFile;
use crate::{document, long_lines, recovery, storage};
use crate::{
    ActualSize, AlignCentre, AlignLeft, AlignRight, CloseAll, CloseBar, CloseWindow, CopyRuler,
    DecreaseFont, DuplicateDocument, FindNext, FindPrev, IncreaseFont, JumpToSelection, NewFile,
    OpenFile, OpenPageSetup, PasteRuler, QuitAndKeepWindows, SaveFile, SaveFileAs, SaveGoToFolder,
    SelectLine, ShowRuler, ShowSettings, ToggleFind, ToggleMono, ToggleReplace, ToggleRichText,
    ToggleWrapToPage, UseSelectionForFind, ZoomIn, ZoomOut,
};

use document_io::{
    can_begin_print, inspect_external_revision, is_rich_text_path, load_selected_document,
    pdf_export_filename, render_pdf_export, save_document, save_document_copy,
    should_reuse_untitled_window, LoadedFile, SaveContent, SaveFailure, SavedDocument,
};
use recovery_state::{recovery_failure_message, startup_recovery, RecoveryClock, RecoveryPrompt};
use startup::{open_duplicate_window, open_editor_window};

pub(crate) use startup::run;

const CTX: &str = "TextEditor";

/// A pending document switch that must wait on an unsaved-changes prompt.
#[derive(Clone, Copy)]
enum Pending {
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SaveLocation {
    #[default]
    Documents,
    Desktop,
    Home,
    Downloads,
    Other,
}

impl SaveLocation {
    fn label(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Desktop => "Desktop",
            Self::Home => "Home",
            Self::Downloads => "Downloads",
            Self::Other => "Other…",
        }
    }

    fn directory(self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        match self {
            Self::Documents => Some(home.join("Documents")),
            Self::Desktop => Some(home.join("Desktop")),
            Self::Home => Some(home),
            Self::Downloads => Some(home.join("Downloads")),
            Self::Other => None,
        }
    }
}

/// The content a duplicate window ([`startup::open_duplicate_window`]) opens
/// with: TextEdit's File ▸ Duplicate copies the buffer exactly, formatting
/// included, into a new window with no path.
pub(super) struct DuplicateContent {
    pub(super) text: String,
    pub(super) format: document::TextFormat,
    pub(super) mono: bool,
    pub(super) font_size: f32,
    /// The rich document, formatting included, when the source is rich.
    pub(super) rich: Option<rich::Document>,
}

/// An edit an assistive technology asks of the document body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssistiveEdit {
    /// Replace the whole document (AccessKit `SetValue`).
    SetValue,
    /// Replace the selection, or insert at the caret (`ReplaceSelectedText`).
    ReplaceSelection,
}

/// A modal alert awaiting the user, shown via the shared `rmac_ui::alert`.
#[derive(Clone)]
enum ActiveAlert {
    /// A recovery file was found — Restore (load it) or Discard.
    Recover(RecoveryPrompt),
    /// The buffer is dirty before `Pending` on a document that has never
    /// been saved (a path-backed one autosaves instead, see `guarded`) —
    /// the Mac's Save sheet: Delete / Cancel / Save.
    ConfirmSave(Option<Pending>),
    /// The opened document no longer matches its retained exact revision.
    Conflict,
    /// The external bytes reviewed immediately before an explicit overwrite.
    ConfirmOverwrite { reviewed_revision: Vec<u8> },
    /// Format ▸ Make Plain Text on a rich document: confirms before
    /// discarding the document's formatting, as TextEdit does.
    ConfirmPlainTextConversion,
    /// File ▸ Revert To ▸ Last Saved: confirms before discarding unsaved
    /// edits, since reverting cannot be undone.
    ConfirmRevert,
    /// A document open/save error — title + message + OK.
    Error {
        title: &'static str,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalChange {
    Modified,
    Missing,
    Unreadable,
}

enum DocumentWatchEvent {
    Changed,
    Unavailable,
}

struct EditorView {
    input: Entity<InputState>,
    /// The rich-text body, shown while `rich_text` is on (TE-03).
    rich: Entity<rich::RichTextEditor>,
    /// The rich document as last saved or opened: the rich dirty baseline.
    /// `None` until a rich document has an exact saved revision.
    saved_rich: Option<rich::Document>,
    path: Option<PathBuf>,
    /// This window's TextEdit-style untitled number ("Untitled", "Untitled
    /// 2", …), held while `path` is `None`. `None` once the document has a
    /// path — it never had a number, or gave its number back.
    untitled_slot: Option<u32>,
    /// Complete bytes read at open or exact successful save readback. Existing
    /// document saves must still match this revision immediately before write.
    saved_bytes: Option<Vec<u8>>,
    text_format: document::TextFormat,
    /// Format at the last exact successful read or write. A format-only
    /// conversion is an unsaved document change just like a text edit.
    saved_format: document::TextFormat,
    /// Text as last saved (or opened/new) — the dirty baseline. A rope
    /// snapshot shares the buffer's chunks, so it costs no copy of the
    /// document, and comparing it stops at the first differing chunk.
    saved_text: Rope,
    /// Bumped whenever the document text changes, keying caches derived
    /// from the whole text.
    text_revision: u64,
    /// `text_revision` at the last save or open: the read-only long-line
    /// view's dirty baseline.
    saved_revision: u64,
    /// The accessible value for `text_revision`, so frames that only move
    /// the caret or blink it do not copy the document.
    accessible_value: Option<(u64, Option<SharedString>)>,
    /// `Some` when the document has a line too long for the editable view;
    /// it is then shown read-only by [`long_line_view`].
    long_lines: Option<long_line_view::LongLineDocument>,
    dirty: bool,
    file_busy: bool,
    print_busy: bool,

    // Find / replace bar
    find_open: bool,
    select_line_open: bool,
    select_line_input: Entity<InputState>,
    replace_mode: bool,
    find_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    save_name_input: Entity<InputState>,
    save_goto_input: Entity<InputState>,
    save_goto_open: bool,
    save_goto_busy: bool,
    save_goto_error: bool,
    save_custom_folder: Option<PathBuf>,
    save_location: SaveLocation,
    /// Byte offsets of every match of the current query in the buffer.
    matches: Vec<usize>,
    /// Index into `matches` of the active match.
    current: usize,

    // Format
    mono: bool,
    font_size: f32,
    wrap_to_page: bool,
    prevent_editing: bool,
    page_width_chars: u16,
    /// Format ▸ Make Rich Text / Make Plain Text (TXT-MENU-075): the body is
    /// the rich-text editor, and Format ▸ Font's styles and Format ▸ Text
    /// apply; the Mac greys those out for a plain-text document.
    rich_text: bool,
    /// View ▸ Zoom for a rich document (its text keeps its own sizes).
    rich_zoom: f32,
    /// Format ▸ Text ▸ Show Ruler: shows the alignment/spacing bar under
    /// the title bar, only while `rich_text` is on.
    show_ruler: bool,
    /// Format ▸ Font ▸ Show Colours.
    colours_open: bool,
    /// Format ▸ List….
    lists_open: bool,
    /// Format ▸ Font ▸ Show Fonts (⌘T): the panel, the installed families
    /// (`None` until the background fontconfig scan returns), and its size
    /// field.
    fonts_open: bool,
    font_families: Option<Vec<SharedString>>,
    font_size_input: Entity<InputState>,
    /// View ▸ Use Dark Background for Windows: a per-window paper-colour
    /// override, independent of the system's light/dark appearance.
    dark_background: bool,

    // File ▸ Rename…
    rename_open: bool,
    rename_input: Entity<InputState>,
    rename_busy: bool,
    rename_error: Option<SharedString>,
    // File ▸ Move To…
    move_busy: bool,
    // File ▸ Page Setup…
    page_setup_open: bool,
    page_setup_letter: bool,
    page_setup_landscape: bool,
    page_setup_before: Option<(bool, bool)>,
    // Format ▸ Text ▸ Spacing…
    spacing_open: bool,
    // File ▸ Show Properties: the sheet and its seven fields, in
    // `format_extras::PROPERTY_FIELDS` order.
    properties_open: bool,
    property_inputs: Vec<Entity<InputState>>,
    // Format ▸ Font ▸ Styles…: the sheet and the document style shown.
    styles_open: bool,
    styles_index: usize,
    // Edit ▸ Link…
    link_open: bool,
    link_input: Entity<InputState>,

    // Infra
    focus: FocusHandle,
    native_window_title: String,
    recovery_directory: PathBuf,
    recovery_path: PathBuf,
    recovery_cleanup_paths: Vec<PathBuf>,
    recovery_clock: RecoveryClock,
    /// Orders the debounced autosave against a session-end flush.
    recovery_writer: recovery::RecoveryWriter,
    recovery_loading: bool,
    /// The window is going away (saved, or the user chose Don't Save), so
    /// its text is no longer unsaved work to keep.
    closing: bool,
    recovery_error: Option<SharedString>,
    status_notice: Option<SharedString>,
    window_generation: u64,
    document_generation: u64,
    current_document_generation: Arc<AtomicU64>,
    external_change: Option<ExternalChange>,
    document_watch_warning: bool,
    watched_directory: Option<PathBuf>,
    document_watcher: Option<notify::RecommendedWatcher>,
    pending_startup_path: Option<PathBuf>,
    pending_open_picker: bool,
    /// The modal alert currently shown, if any (shared `rmac_ui::alert`).
    alert: Option<ActiveAlert>,
    _subscriptions: Vec<Subscription>,
    /// Edit ▸ Spelling and Grammar / Substitutions for the document.
    text_assist: rmac_ui::text_assist::TextAssistSettings,
    spell_checker: Arc<rmac_spelling::HunspellChecker>,
    /// Edit ▸ Substitutions ▸ Data Detectors: checked, persisted for this
    /// session, with no live effect wired yet (see docs/parity.md TE-07).
    data_detectors: bool,
}

#[cfg(test)]
mod tests {
    use super::document_io::same_file_identity;
    use super::recovery_state::recovery_path_for_platform;
    use super::{
        can_begin_print, document, pdf_export_filename, render_pdf_export, save_document,
        save_document_copy, should_reuse_untitled_window, RecoveryClock, SaveContent, SaveFailure,
        SavedDocument,
    };
    use rmac_editor::rich;
    use std::path::PathBuf;

    #[test]
    fn clean_transition_invalidates_a_pending_recovery_write() {
        let mut clock = RecoveryClock::default();
        let pending = clock.arm();

        assert!(clock.should_write(pending, true));
        assert!(!clock.should_write(pending, false));

        clock.invalidate();
        assert!(!clock.should_write(pending, true));
    }

    #[test]
    fn newer_edit_invalidates_an_older_recovery_write() {
        let mut clock = RecoveryClock::default();
        let older = clock.arm();
        let newer = clock.arm();

        assert!(!clock.should_write(older, true));
        assert!(clock.should_write(newer, true));
    }

    // Unix path literals: on Windows `/var/state` is not absolute.
    #[cfg(unix)]
    #[test]
    fn recovery_paths_follow_xdg_and_macos_conventions() {
        let linux_xdg = recovery_path_for_platform(
            false,
            Some(PathBuf::from("/var/state")),
            Some(PathBuf::from("/home/user")),
        )
        .unwrap();
        let linux_fallback = recovery_path_for_platform(
            false,
            Some(PathBuf::from("relative-state")),
            Some(PathBuf::from("/home/user")),
        )
        .unwrap();
        let macos = recovery_path_for_platform(
            true,
            Some(PathBuf::from("/ignored")),
            Some(PathBuf::from("/Users/user")),
        )
        .unwrap();

        assert_eq!(
            linux_xdg,
            PathBuf::from("/var/state/rmac-text-editor/recovery.txt")
        );
        assert_eq!(
            linux_fallback,
            PathBuf::from("/home/user/.local/state/rmac-text-editor/recovery.txt")
        );
        assert_eq!(
            macos,
            PathBuf::from("/Users/user/Library/Application Support/rmac-text-editor/recovery.txt")
        );
    }

    #[test]
    fn identical_copy_destination_is_rejected_even_before_it_exists() {
        let path = PathBuf::from("document.txt");
        assert!(same_file_identity(&path, &path));
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_copy_destination_is_the_same_file_identity() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-text-editor-copy-identity-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source.txt");
        let link = directory.join("link.txt");
        std::fs::write(&source, b"external revision").unwrap();
        std::fs::hard_link(&source, &link).unwrap();

        assert!(same_file_identity(&source, &link));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_copy_never_replaces_its_forbidden_source() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-text-editor-copy-protection-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source.txt");
        std::fs::write(&source, b"external revision").unwrap();

        let error = save_document_copy(
            &source,
            Some(&source),
            &SaveContent::Plain("local buffer".into()),
            document::TextFormat::default(),
        )
        .unwrap_err();

        assert!(matches!(error, SaveFailure::ConflictingCopyDestination));
        assert_eq!(std::fs::read(&source).unwrap(), b"external revision");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_reuses_only_a_clean_empty_untitled_window() {
        assert!(should_reuse_untitled_window(false, false, true));
        assert!(!should_reuse_untitled_window(true, false, true));
        assert!(!should_reuse_untitled_window(false, true, true));
        assert!(!should_reuse_untitled_window(false, false, false));
    }

    #[test]
    fn rich_documents_save_as_rtf_and_read_back_with_their_formatting() {
        let directory =
            std::env::temp_dir().join(format!("rmac-text-editor-rich-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("styled.rtf");

        // Type "Hello world", select "world", ⌘B, centre the paragraph.
        let mut rich_document =
            rich::Document::from_plain_text("Hello world", &rich::CharStyle::default());
        rich_document.update_char_style(6..11, |style| style.bold = true);
        rich_document
            .update_paragraph_style(0..0, |style| style.alignment = rich::Alignment::Center);
        let saved = save_document(
            &path,
            None,
            &SaveContent::Rich(rich_document.clone()),
            document::TextFormat::default(),
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            matches!(saved, SavedDocument::Rich { ref original_bytes } if *original_bytes == bytes)
        );
        assert!(bytes.starts_with(b"{\\rtf1"));
        assert!(String::from_utf8_lossy(&bytes).contains("\\b world"));

        let loaded = super::load_selected_document(&path).unwrap();
        let super::LoadedFile::RichText {
            document: reopened, ..
        } = loaded
        else {
            panic!("an .rtf opens as rich text");
        };
        assert_eq!(reopened, rich_document);
        assert!(reopened.style_of_char_at(6).bold);
        assert!(!reopened.style_of_char_at(0).bold);
        assert_eq!(
            reopened.paragraph(0).style().alignment,
            rich::Alignment::Center
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rich_documents_print_with_their_formatting_markers_and_breaks() {
        let mut rich_document =
            rich::Document::from_plain_text("Title\nitem\u{2028}more", &rich::CharStyle::default());
        rich_document.update_char_style(0..5, |style| {
            style.bold = true;
            style.color = Some(rich::Rgb::new(200, 0, 0));
        });
        rich_document
            .update_paragraph_style(0..0, |style| style.alignment = rich::Alignment::Center);
        rich_document
            .update_paragraph_style(6..6, |style| style.list = Some(rich::ListKind::Numbered));
        let lines = super::document_io::print_lines(&rich_document);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].align, rmac_print::RichAlign::Center);
        assert!(lines[0].spans[0].bold);
        assert_eq!(lines[0].spans[0].color, (200, 0, 0));
        assert_eq!(lines[1].spans[0].text, "1.\t");
        assert_eq!(lines[1].spans[1].text, "item");
        assert_eq!(lines[2].spans[0].text, "more");
        let pdf = rmac_print::render_rich_pdf(&lines, rmac_print::PageLayout::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn printing_requires_one_stable_plain_text_window_state() {
        assert!(can_begin_print(false, false, false, false, false));
        assert!(!can_begin_print(true, false, false, false, false));
        assert!(!can_begin_print(false, true, false, false, false));
        assert!(!can_begin_print(false, false, true, false, false));
        assert!(!can_begin_print(false, false, false, true, false));
        assert!(!can_begin_print(false, false, false, false, true));
    }

    #[test]
    fn pdf_export_replaces_the_extension_or_names_an_untitled_document() {
        assert_eq!(
            pdf_export_filename(Some(&PathBuf::from("/home/user/notes.txt"))),
            "notes.pdf"
        );
        assert_eq!(
            pdf_export_filename(Some(&PathBuf::from("draft.rtf"))),
            "draft.pdf"
        );
        // No extension at all: the whole name is the stem.
        assert_eq!(
            pdf_export_filename(Some(&PathBuf::from("README"))),
            "README.pdf"
        );
        assert_eq!(pdf_export_filename(None), "Untitled.pdf");
    }

    #[test]
    fn export_pdf_writes_a_valid_pdf_to_the_chosen_path() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-text-editor-pdf-export-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("export.pdf");

        render_pdf_export(
            &path,
            "Exported from Text Editor",
            rmac_print::PageLayout::default(),
        )
        .unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF"));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
