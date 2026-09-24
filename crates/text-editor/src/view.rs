//! Text Editor startup, document-window, and interaction controller.

mod alerts;
mod conflicts;
mod document_io;
mod document_state;
mod editing;
mod lifecycle;
mod long_line_view;
mod opening;
mod printing;
mod recovery_state;
mod render;
mod responsive_layout;
mod saving;
mod startup;

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
use rmac_ui::{InputEvent, InputState, Position, Rope, RopeExt as _};

#[cfg(target_os = "linux")]
use crate::PrintFile;
use crate::{document, long_lines, recovery, rtf, storage};
use crate::{
    CloseBar, CloseWindow, DecreaseFont, FindNext, FindPrev, IncreaseFont, NewFile, OpenFile,
    SaveFile, SaveFileAs, ToggleFind, ToggleMono, ToggleReplace,
};

use document_io::{
    can_begin_print, inspect_external_revision, load_selected_document, save_document,
    save_document_copy, should_reuse_untitled_window, LoadedFile, SaveFailure,
};
use recovery_state::{recovery_failure_message, startup_recovery, RecoveryClock, RecoveryPrompt};
use startup::open_editor_window;

pub(crate) use startup::run;

const CTX: &str = "TextEditor";

/// A pending document switch that must wait on an unsaved-changes prompt.
#[derive(Clone, Copy)]
enum Pending {
    Close,
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
    /// The buffer is dirty before `Pending` — Save / Don't Save / Cancel.
    ConfirmSave(Pending),
    /// The opened document no longer matches its retained exact revision.
    Conflict,
    /// The external bytes reviewed immediately before an explicit overwrite.
    ConfirmOverwrite { reviewed_revision: Vec<u8> },
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
    path: Option<PathBuf>,
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
    replace_mode: bool,
    find_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    /// Byte offsets of every match of the current query in the buffer.
    matches: Vec<usize>,
    /// Index into `matches` of the active match.
    current: usize,

    // Format
    mono: bool,
    font_size: f32,

    /// When an `.rtf` is opened, its parsed styled runs for the formatted
    /// preview. `Some` puts the editor in read-only RTF-viewer mode.
    rtf_runs: Option<Vec<rtf::RtfRun>>,

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
    /// The modal alert currently shown, if any (shared `rmac_ui::alert`).
    alert: Option<ActiveAlert>,
    _subscriptions: Vec<Subscription>,
}

#[cfg(test)]
mod tests {
    use super::document_io::same_file_identity;
    use super::recovery_state::recovery_path_for_platform;
    use super::{
        can_begin_print, document, save_document_copy, should_reuse_untitled_window, RecoveryClock,
        SaveFailure,
    };
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
            "local buffer",
            document::TextFormat::default(),
        )
        .unwrap_err();

        assert!(matches!(error, SaveFailure::ConflictingCopyDestination));
        assert_eq!(std::fs::read(&source).unwrap(), b"external revision");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_reuses_only_a_clean_empty_untitled_window() {
        assert!(should_reuse_untitled_window(false, false, false, true));
        assert!(!should_reuse_untitled_window(true, false, false, false));
        assert!(!should_reuse_untitled_window(false, true, false, false));
        assert!(!should_reuse_untitled_window(false, false, true, false));
        assert!(!should_reuse_untitled_window(false, false, false, false));
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
}
