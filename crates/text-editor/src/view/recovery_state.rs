//! Text Editor recovery clock, discovery, claiming, and legacy migration authority.

use super::*;

static STARTUP_RECOVERY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
pub(super) struct RecoveryClock {
    generation: u64,
}

impl RecoveryClock {
    pub(super) fn arm(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn should_write(&self, generation: u64, dirty: bool) -> bool {
        dirty && self.generation == generation
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

#[derive(Clone)]
pub(super) struct RecoveryPrompt {
    pub(super) content: String,
    pub(super) document_label: String,
    pub(super) format: document::TextFormat,
    pub(super) additional_drafts: usize,
}

fn platform_recovery_path() -> Result<PathBuf, storage::Failure> {
    recovery_path_for_platform(
        cfg!(target_os = "macos"),
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

pub(super) fn recovery_path_for_platform(
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

pub(super) struct StartupRecovery {
    pub(super) directory: PathBuf,
    pub(super) active_path: PathBuf,
    pub(super) cleanup_paths: Vec<PathBuf>,
    pub(super) prompt: Option<RecoveryPrompt>,
    pub(super) warning: bool,
}

pub(super) fn startup_recovery() -> StartupRecovery {
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

pub(super) fn recovery_failure_message() -> SharedString {
    "Text Editor could not safely update its private recovery data. The current buffer remains open; save the document before closing."
        .into()
}
