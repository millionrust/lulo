//! Text Editor startup, document-window, and interaction controller.

mod alerts;
mod conflicts;
mod document_io;
mod document_state;
mod editing;
mod lifecycle;
mod opening;
mod printing;
mod render;
mod responsive_layout;
mod saving;

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
    App, AppContext as _, Application, Context, Entity, FocusHandle, KeyBinding, PathPromptOptions,
    SharedString, Subscription, Window,
};
use gpui_component::Root;
use notify::Watcher as _;
use rmac_ui::{InputEvent, InputState, Position, RopeExt as _};

#[cfg(target_os = "linux")]
use crate::PrintFile;
use crate::{document, recovery, rtf, storage};
use crate::{
    CloseBar, CloseWindow, DecreaseFont, FindNext, FindPrev, IncreaseFont, NewFile, OpenFile,
    SaveFile, SaveFileAs, ToggleFind, ToggleMono, ToggleReplace,
};

use document_io::{
    can_begin_print, inspect_external_revision, load_selected_document, save_document,
    save_document_copy, should_reuse_untitled_window, LoadedFile, SaveFailure,
};

const CTX: &str = "TextEditor";
const WINDOW_WIDTH: f32 = 860.0;
const WINDOW_HEIGHT: f32 = 640.0;
const MAX_STARTUP_DOCUMENTS: usize = 32;
static STARTUP_RECOVERY_LOCK: Mutex<()> = Mutex::new(());
static NEXT_WINDOW_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Eq, PartialEq)]
struct StartupRequest {
    open_untitled: bool,
    paths: Vec<PathBuf>,
}

fn parse_startup_request(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<StartupRequest, &'static str> {
    let mut paths = Vec::new();
    let mut open_untitled = false;
    let mut options = true;
    for argument in arguments {
        if options && argument == "--" {
            options = false;
            continue;
        }
        if options && argument == "--new-document" {
            open_untitled = true;
            continue;
        }
        if options
            && argument
                .to_str()
                .is_some_and(|value| value.starts_with('-'))
        {
            return Err("unsupported Text Editor launch option");
        }
        let path = PathBuf::from(argument);
        if path.as_os_str().is_empty() {
            return Err("empty Text Editor launch path");
        }
        if paths.len() == MAX_STARTUP_DOCUMENTS {
            return Err("too many Text Editor startup documents");
        }
        paths.push(path);
    }
    if paths.is_empty() && !open_untitled {
        open_untitled = true;
    }
    Ok(StartupRequest {
        open_untitled,
        paths,
    })
}

/// A pending document switch that must wait on an unsaved-changes prompt.
#[derive(Clone, Copy)]
enum Pending {
    Close,
}

#[derive(Default)]
struct RecoveryClock {
    generation: u64,
}

impl RecoveryClock {
    fn arm(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn should_write(&self, generation: u64, dirty: bool) -> bool {
        dirty && self.generation == generation
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
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

#[derive(Clone)]
struct RecoveryPrompt {
    content: String,
    document_label: String,
    format: document::TextFormat,
    additional_drafts: usize,
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
    /// Text as last saved (or opened/new) — the dirty baseline.
    saved_value: String,
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
    recovery_loading: bool,
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

fn open_editor_window(cx: &mut App, initial_path: Option<PathBuf>) -> Result<(), ()> {
    let options = rmac_ui::window_options_for_app(
        rmac_ui::app_id::TEXT_EDITOR,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        cx,
    );
    cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            rmac_ui::observe_window_state(rmac_ui::app_id::TEXT_EDITOR, window, cx);
            EditorView::new_with_path(initial_path, window, cx)
        });
        cx.new(|cx| Root::new(view, window, cx))
    })
    .map(|_| ())
    .map_err(|_| ())
}

fn platform_recovery_path() -> Result<PathBuf, storage::Failure> {
    recovery_path_for_platform(
        cfg!(target_os = "macos"),
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn recovery_path_for_platform(
    macos: bool,
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf, storage::Failure> {
    if !macos {
        if let Some(path) = xdg_state_home.filter(|path| path.is_absolute()) {
            return Ok(path.join("rmac-text-editor/recovery.txt"));
        }
    }

    let home = home.ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveRecoveryPath,
            Path::new("recovery.txt"),
            "HOME is not set and XDG_STATE_HOME is unavailable",
        )
    })?;
    if macos {
        Ok(home.join("Library/Application Support/rmac-text-editor/recovery.txt"))
    } else {
        Ok(home.join(".local/state/rmac-text-editor/recovery.txt"))
    }
}

struct StartupRecovery {
    directory: PathBuf,
    active_path: PathBuf,
    cleanup_paths: Vec<PathBuf>,
    prompt: Option<RecoveryPrompt>,
    warning: bool,
}

fn startup_recovery() -> StartupRecovery {
    // Multiple windows can launch concurrently. Serializing only this
    // background discovery/migration boundary prevents two windows from
    // importing the same legacy raw draft before either removes it.
    let _guard = STARTUP_RECOVERY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary_legacy = std::env::temp_dir().join("rmac-text-editor-recovery.txt");
    let (legacy_primary, mut warning) = match platform_recovery_path() {
        Ok(path) => (path, false),
        Err(_) => (temporary_legacy.clone(), true),
    };
    let directory = legacy_primary
        .parent()
        .map(|parent| parent.join("recovery"))
        .unwrap_or_else(|| std::env::temp_dir().join("rmac-text-editor-recovery"));
    let mut discovery = recovery::discover(&directory);
    warning |= discovery.unavailable || discovery.excessive || discovery.malformed > 0;

    let loaded_legacy = storage::load_legacy_recovery_drafts(
        &storage::RealStorage,
        &legacy_primary,
        &temporary_legacy,
    );
    warning |= loaded_legacy.warning.is_some();
    let mut cleanup_paths = Vec::new();
    let mut unmigrated = Vec::new();
    let legacy_count = loaded_legacy.drafts.len();
    for (index, draft) in loaded_legacy.drafts.into_iter().enumerate() {
        let record = recovery::RecoveryRecord::for_document(
            None,
            document::TextFormat::default(),
            draft.content,
        );
        let migrated_path = recovery::fresh_record_path(&directory);
        if recovery::save(&storage::RealStorage, &migrated_path, &record).is_ok() {
            if storage::remove_recovery_paths(&storage::RealStorage, &draft.paths).is_err() {
                warning = true;
                cleanup_paths.extend(draft.paths);
            }
            discovery.candidates.push(recovery::Candidate {
                path: migrated_path,
                record,
            });
        } else {
            warning = true;
            unmigrated.push((
                RecoveryPrompt {
                    content: record.content,
                    document_label: if legacy_count == 1 {
                        "Legacy unsaved document".into()
                    } else {
                        format!("Legacy unsaved document {}", index + 1)
                    },
                    format: record.format,
                    additional_drafts: 0,
                },
                draft.paths,
            ));
        }
    }

    discovery.candidates.sort_by(|left, right| {
        right
            .record
            .created_unix_ms
            .cmp(&left.record.created_unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    if !unmigrated.is_empty() {
        let additional_drafts = discovery
            .candidates
            .len()
            .saturating_add(unmigrated.len().saturating_sub(1));
        let (mut prompt, selected_paths) = unmigrated.remove(0);
        prompt.additional_drafts = additional_drafts;
        cleanup_paths.extend(selected_paths);
        let active_path = recovery::fresh_record_path(&directory);
        return StartupRecovery {
            directory,
            active_path,
            cleanup_paths,
            prompt: Some(prompt),
            warning,
        };
    }
    let mut selected = None;
    let mut remaining = discovery.candidates.len();
    for candidate in discovery.candidates {
        remaining = remaining.saturating_sub(1);
        match recovery::claim(&directory, candidate) {
            Ok(claimed) => {
                selected = Some(claimed);
                break;
            }
            Err(_) => warning = true,
        }
    }
    let active_path = selected
        .as_ref()
        .map(|candidate| candidate.path.clone())
        .unwrap_or_else(|| recovery::fresh_record_path(&directory));
    let prompt = selected.map(|candidate| RecoveryPrompt {
        content: candidate.record.content,
        document_label: candidate.record.document_label,
        format: candidate.record.format,
        additional_drafts: remaining,
    });
    StartupRecovery {
        directory,
        active_path,
        cleanup_paths,
        prompt,
        warning,
    }
}

fn recovery_failure_message() -> SharedString {
    "Text Editor could not safely update its private recovery data. The current buffer remains open; save the document before closing."
        .into()
}
pub(crate) fn run() {
    let request = match parse_startup_request(std::env::args_os().skip(1)) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            if request.open_untitled && open_editor_window(cx, None).is_err() {
                eprintln!("Text Editor could not open a document window");
            }
            for path in request.paths {
                if open_editor_window(cx, Some(path)).is_err() {
                    eprintln!("Text Editor could not open a document window");
                }
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::document_io::same_file_identity;
    use super::{
        can_begin_print, document, parse_startup_request, recovery_path_for_platform,
        save_document_copy, should_reuse_untitled_window, RecoveryClock, SaveFailure,
        StartupRequest, MAX_STARTUP_DOCUMENTS,
    };
    use std::ffi::OsString;
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
    fn desktop_launch_paths_open_independent_windows() {
        assert_eq!(
            parse_startup_request([
                OsString::from("/home/user/one.txt"),
                OsString::from("/home/user/two.rtf"),
            ])
            .unwrap(),
            StartupRequest {
                open_untitled: false,
                paths: vec![
                    PathBuf::from("/home/user/one.txt"),
                    PathBuf::from("/home/user/two.rtf"),
                ],
            }
        );
        assert_eq!(
            parse_startup_request([OsString::from("--new-document")]).unwrap(),
            StartupRequest {
                open_untitled: true,
                paths: Vec::new(),
            }
        );
    }

    #[test]
    fn startup_arguments_are_bounded_and_options_fail_closed() {
        assert!(parse_startup_request([OsString::from("--unknown")]).is_err());
        assert_eq!(
            parse_startup_request([OsString::from("--"), OsString::from("-literal-name.txt"),])
                .unwrap()
                .paths,
            vec![PathBuf::from("-literal-name.txt")]
        );
        assert!(parse_startup_request(
            (0..=MAX_STARTUP_DOCUMENTS).map(|index| OsString::from(format!("/tmp/{index}")))
        )
        .is_err());
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
