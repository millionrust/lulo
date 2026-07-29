use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::file_ops::{self, CopyActivity, FileSystem};
use crate::operation_journal::{EntryIdentity, TreeSnapshot};

const UNDO_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_RECORDS: usize = 512;
const RETAIN_READY_RECORDS: usize = 20;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UndoKind {
    Copy,
    Move,
    Replace,
    MoveReplace,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UndoStage {
    ForwardPending,
    Ready,
    ReplacementRestored,
    SourceCopying,
    SourceCopyComplete,
    SourceRestored,
    CleanupStaged,
}

pub(crate) struct UndoSeed {
    pub(crate) id: String,
    pub(crate) kind: UndoKind,
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) backup: Option<PathBuf>,
    pub(crate) source_snapshot: TreeSnapshot,
    pub(crate) destination_snapshot: TreeSnapshot,
    pub(crate) replaced_snapshot: Option<TreeSnapshot>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct UndoRecord {
    version: u32,
    id: String,
    kind: UndoKind,
    stage: UndoStage,
    created_seconds: u64,
    created_nanoseconds: u32,
    source_path_bytes: Vec<u8>,
    destination_path_bytes: Vec<u8>,
    backup_path_bytes: Option<Vec<u8>>,
    restore_staging_path_bytes: Vec<u8>,
    cleanup_path_bytes: Vec<u8>,
    source_parent_identity: EntryIdentity,
    source_snapshot: TreeSnapshot,
    destination_snapshot: TreeSnapshot,
    replaced_snapshot: Option<TreeSnapshot>,
    restore_container_identity: Option<EntryIdentity>,
    restored_snapshot: Option<TreeSnapshot>,
}

impl UndoRecord {
    fn from_seed(seed: UndoSeed) -> io::Result<Self> {
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid_data("system clock precedes the Unix epoch"))?;
        let source_parent = seed
            .source
            .parent()
            .ok_or_else(|| invalid_data("undo source has no parent"))?;
        let destination_parent = seed
            .destination
            .parent()
            .ok_or_else(|| invalid_data("undo destination has no parent"))?;
        let id = seed.id;
        let restore_staging = source_parent.join(format!(".rmac-undo-restore-{id}"));
        let cleanup = destination_parent.join(format!(".rmac-undo-remove-{id}"));
        let record = Self {
            version: UNDO_VERSION,
            id,
            kind: seed.kind,
            stage: UndoStage::ForwardPending,
            created_seconds: created.as_secs(),
            created_nanoseconds: created.subsec_nanos(),
            source_path_bytes: seed.source.as_os_str().as_bytes().to_vec(),
            destination_path_bytes: seed.destination.as_os_str().as_bytes().to_vec(),
            backup_path_bytes: seed.backup.map(|path| path.as_os_str().as_bytes().to_vec()),
            restore_staging_path_bytes: restore_staging.as_os_str().as_bytes().to_vec(),
            cleanup_path_bytes: cleanup.as_os_str().as_bytes().to_vec(),
            source_parent_identity: EntryIdentity::capture(source_parent)?,
            source_snapshot: seed.source_snapshot,
            destination_snapshot: seed.destination_snapshot,
            replaced_snapshot: seed.replaced_snapshot,
            restore_container_identity: None,
            restored_snapshot: None,
        };
        record.validate(&record.id)?;
        Ok(record)
    }

    fn source(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.source_path_bytes.clone()))
    }

    fn destination(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.destination_path_bytes.clone()))
    }

    fn backup(&self) -> Option<PathBuf> {
        self.backup_path_bytes
            .as_ref()
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes.clone())))
    }

    fn restore_staging(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.restore_staging_path_bytes.clone()))
    }

    fn restore_payload(&self) -> PathBuf {
        self.restore_staging().join("payload")
    }

    fn cleanup(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.cleanup_path_bytes.clone()))
    }

    fn matches_seed(&self, seed: &UndoSeed) -> bool {
        self.id == seed.id
            && self.kind == seed.kind
            && self.source() == seed.source
            && self.destination() == seed.destination
            && self.backup() == seed.backup
            && self.source_snapshot == seed.source_snapshot
            && self.destination_snapshot == seed.destination_snapshot
            && self.replaced_snapshot == seed.replaced_snapshot
    }

    fn validate(&self, expected_id: &str) -> io::Result<()> {
        if self.version != UNDO_VERSION
            || self.id != expected_id
            || Uuid::parse_str(&self.id).is_err()
        {
            return Err(invalid_data("undo receipt identity or version is invalid"));
        }
        let source = self.source();
        let destination = self.destination();
        let restore = self.restore_staging();
        let cleanup = self.cleanup();
        if !source.is_absolute()
            || !destination.is_absolute()
            || !restore.is_absolute()
            || !cleanup.is_absolute()
            || source == destination
            || source.parent() != restore.parent()
            || destination.parent() != cleanup.parent()
            || restore.file_name().and_then(|name| name.to_str())
                != Some(format!(".rmac-undo-restore-{}", self.id).as_str())
            || cleanup.file_name().and_then(|name| name.to_str())
                != Some(format!(".rmac-undo-remove-{}", self.id).as_str())
        {
            return Err(invalid_data("undo receipt path relationship is invalid"));
        }
        let replacement = matches!(self.kind, UndoKind::Replace | UndoKind::MoveReplace);
        if replacement != (self.backup_path_bytes.is_some() && self.replaced_snapshot.is_some()) {
            return Err(invalid_data(
                "undo replacement evidence is missing or unexpected",
            ));
        }
        if let Some(backup) = self.backup() {
            if !backup.is_absolute()
                || backup.parent() != destination.parent()
                || backup.file_name().and_then(|name| name.to_str())
                    != Some(format!(".rmac-transfer-{}", self.id).as_str())
                || backup == cleanup
            {
                return Err(invalid_data("undo replacement backup path is invalid"));
            }
        }
        match self.kind {
            UndoKind::Copy
                if !matches!(
                    self.stage,
                    UndoStage::ForwardPending | UndoStage::Ready | UndoStage::CleanupStaged
                ) =>
            {
                return Err(invalid_data("copy undo has an impossible stage"));
            }
            UndoKind::Replace
                if !matches!(
                    self.stage,
                    UndoStage::ForwardPending
                        | UndoStage::Ready
                        | UndoStage::ReplacementRestored
                        | UndoStage::CleanupStaged
                ) =>
            {
                return Err(invalid_data("replacement undo has an impossible stage"));
            }
            UndoKind::Move if matches!(self.stage, UndoStage::ReplacementRestored) => {
                return Err(invalid_data("move undo has an impossible stage"));
            }
            UndoKind::MoveReplace if self.stage == UndoStage::Ready => {}
            _ => {}
        }
        if matches!(
            self.stage,
            UndoStage::ForwardPending | UndoStage::Ready | UndoStage::ReplacementRestored
        ) && (self.restore_container_identity.is_some() || self.restored_snapshot.is_some())
        {
            return Err(invalid_data(
                "undo source-copy evidence appears before copying",
            ));
        }
        if self.stage == UndoStage::SourceCopying && self.restored_snapshot.is_some() {
            return Err(invalid_data(
                "undo source-copy intent unexpectedly has completed evidence",
            ));
        }
        if matches!(
            self.stage,
            UndoStage::SourceCopyComplete | UndoStage::SourceRestored
        ) && (self.restore_container_identity.is_none() || self.restored_snapshot.is_none())
        {
            return Err(invalid_data("undo restored-source evidence is incomplete"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UndoStore {
    root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UndoAvailability {
    pub(crate) label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UndoOutcome {
    pub(crate) label: String,
}

impl UndoStore {
    pub(crate) fn open(root: PathBuf) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "undo root must be absolute",
            ));
        }
        fs::create_dir_all(&root)?;
        let metadata = fs::symlink_metadata(&root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid_data("undo root is not a real directory"));
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let store = Self { root };
        let _lock = store.acquire_lock()?;
        store.remove_abandoned_temps()?;
        Ok(store)
    }

    pub(crate) fn archive(&self, seed: UndoSeed) -> io::Result<()> {
        let _lock = self.acquire_lock()?;
        let path = self.record_path(&seed.id);
        match self.read_record_path(&path) {
            Ok(existing) => {
                if !existing.matches_seed(&seed) {
                    return Err(invalid_data(
                        "existing undo receipt does not match the completed transfer",
                    ));
                }
                return self.prune_ready();
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.prune_ready()?;
        if self.read_records()?.len() >= MAX_RECORDS {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "Undo history is full because retained replacement data could not be pruned safely",
            ));
        }
        let record = UndoRecord::from_seed(seed)?;
        if let Some(backup) = record.backup() {
            let matches = record
                .replaced_snapshot
                .as_ref()
                .ok_or_else(|| invalid_data("replacement receipt has no prior-item evidence"))?
                .still_matches_after_rename(&backup)?;
            if !matches {
                return Err(invalid_data(
                    "replacement backup changed before Undo was archived",
                ));
            }
        }
        self.persist(&record, true)?;
        self.prune_ready()
    }

    pub(crate) fn latest(&self) -> io::Result<Option<UndoAvailability>> {
        let _lock = self.acquire_lock()?;
        self.promote_detached_forward_receipts()?;
        let Some(record) = self.latest_record()? else {
            return Ok(None);
        };
        Ok(Some(UndoAvailability {
            label: undo_label(&record),
        }))
    }

    pub(crate) fn execute_latest(
        &self,
        fs: &impl FileSystem,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<Option<UndoOutcome>> {
        let _lock = self.acquire_lock()?;
        self.promote_detached_forward_receipts()?;
        let Some(mut record) = self.latest_record()? else {
            return Ok(None);
        };
        let label = undo_label(&record);
        self.resume_inferred(&mut record)?;
        if !self.record_path(&record.id).exists() {
            return Ok(Some(UndoOutcome { label }));
        }
        if cancel.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        match record.kind {
            UndoKind::Copy => self.undo_copy(&mut record, cancel, progress)?,
            UndoKind::Move => self.undo_move(fs, &mut record, cancel, progress)?,
            UndoKind::Replace => self.undo_replace(&mut record, cancel, progress)?,
            UndoKind::MoveReplace => self.undo_move_replace(fs, &mut record, cancel, progress)?,
        }
        Ok(Some(UndoOutcome { label }))
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> io::Result<usize> {
        let _lock = self.acquire_lock()?;
        Ok(self.read_records()?.len())
    }

    fn latest_record(&self) -> io::Result<Option<UndoRecord>> {
        Ok(self
            .read_records()?
            .into_iter()
            .filter(|record| record.stage != UndoStage::ForwardPending)
            .max_by_key(|record| {
                (
                    record.stage != UndoStage::Ready,
                    record.created_seconds,
                    record.created_nanoseconds,
                    record.id.clone(),
                )
            }))
    }

    pub(crate) fn activate(&self, id: &str) -> io::Result<()> {
        let _lock = self.acquire_lock()?;
        let path = self.record_path(id);
        let mut record = self.read_record_path(&path)?;
        if record.stage == UndoStage::Ready {
            return Ok(());
        }
        if record.stage != UndoStage::ForwardPending {
            return Err(invalid_data(
                "undo receipt advanced before its forward operation committed",
            ));
        }
        if self.forward_record_path(id).exists() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "forward operation is still committing",
            ));
        }
        record.stage = UndoStage::Ready;
        self.persist(&record, false)?;
        self.prune_ready()
    }

    fn promote_detached_forward_receipts(&self) -> io::Result<()> {
        for mut record in self.read_records()? {
            if record.stage == UndoStage::ForwardPending
                && !self.forward_record_path(&record.id).exists()
            {
                record.stage = UndoStage::Ready;
                self.persist(&record, false)?;
            }
        }
        self.prune_ready()
    }

    fn forward_record_path(&self, id: &str) -> PathBuf {
        self.root
            .parent()
            .unwrap_or(&self.root)
            .join(format!("{id}.json"))
    }

    fn resume_inferred(&self, record: &mut UndoRecord) -> io::Result<()> {
        if record.stage == UndoStage::SourceCopying {
            self.discard_partial_restore(record)?;
        }

        if record.stage == UndoStage::SourceCopyComplete {
            let restored = record
                .restored_snapshot
                .as_ref()
                .ok_or_else(|| invalid_data("completed source copy has no evidence"))?;
            let source = record.source();
            let payload = record.restore_payload();
            if restored.still_matches_after_rename(&source)? && !entry_exists(&payload)? {
                self.remove_restore_container(record)?;
                record.stage = UndoStage::SourceRestored;
                self.persist(record, false)?;
            }
        }

        if record.stage == UndoStage::Ready {
            match record.kind {
                UndoKind::Copy => {
                    if !entry_exists(&record.destination())?
                        && record
                            .destination_snapshot
                            .still_matches_after_rename(&record.cleanup())?
                    {
                        record.stage = UndoStage::CleanupStaged;
                        self.persist(record, false)?;
                    } else if !entry_exists(&record.destination())?
                        && !entry_exists(&record.cleanup())?
                    {
                        self.finish(record)?;
                        return Ok(());
                    }
                }
                UndoKind::Move => {
                    if record
                        .source_snapshot
                        .still_matches_after_rename(&record.source())?
                        && !entry_exists(&record.destination())?
                    {
                        self.finish(record)?;
                        return Ok(());
                    }
                }
                UndoKind::Replace | UndoKind::MoveReplace => {
                    let backup = record
                        .backup()
                        .ok_or_else(|| invalid_data("replacement undo has no backup"))?;
                    let old_at_destination = record
                        .replaced_snapshot
                        .as_ref()
                        .ok_or_else(|| invalid_data("replacement undo has no prior-item evidence"))?
                        .still_matches_after_rename(&record.destination())?;
                    let new_at_backup = record
                        .destination_snapshot
                        .still_matches_after_rename(&backup)?;
                    if old_at_destination && new_at_backup {
                        record.stage = UndoStage::ReplacementRestored;
                        self.persist(record, false)?;
                    }
                }
            }
        }

        if record.stage == UndoStage::ReplacementRestored && record.kind == UndoKind::MoveReplace {
            let backup = record
                .backup()
                .ok_or_else(|| invalid_data("move replacement undo has no backup"))?;
            if record
                .destination_snapshot
                .still_matches_after_rename(&record.source())?
                && !entry_exists(&backup)?
            {
                self.finish(record)?;
                return Ok(());
            }
        }

        if record.stage == UndoStage::CleanupStaged {
            let cleanup = record.cleanup();
            if entry_exists(&cleanup)? {
                remove_bound_tree(&cleanup, &record.destination_snapshot)?;
            }
            if !entry_exists(&cleanup)? {
                self.finish(record)?;
            }
        } else {
            let cleanup_source = match (record.kind, record.stage) {
                (UndoKind::Replace, UndoStage::ReplacementRestored)
                | (UndoKind::MoveReplace, UndoStage::SourceRestored) => record.backup(),
                (UndoKind::Move, UndoStage::SourceRestored) => Some(record.destination()),
                _ => None,
            };
            if let Some(source) = cleanup_source {
                if !entry_exists(&source)?
                    && record
                        .destination_snapshot
                        .still_matches_after_rename(&record.cleanup())?
                {
                    record.stage = UndoStage::CleanupStaged;
                    self.persist(record, false)?;
                    remove_bound_tree(&record.cleanup(), &record.destination_snapshot)?;
                    self.finish(record)?;
                }
            }
        }
        Ok(())
    }

    fn undo_copy(
        &self,
        record: &mut UndoRecord,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        if record.stage == UndoStage::CleanupStaged {
            return self.finish_cleanup(record);
        }
        if record.stage != UndoStage::Ready {
            return Err(invalid_data("copy undo is in an unsupported state"));
        }
        if !record.source_snapshot.still_matches(&record.source())? {
            return Err(changed());
        }
        if cancel.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        self.stage_cleanup(record, &record.destination())?;
        progress(CopyActivity::Finishing);
        self.finish_cleanup(record)
    }

    fn undo_replace(
        &self,
        record: &mut UndoRecord,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        if record.stage == UndoStage::CleanupStaged {
            return self.finish_cleanup(record);
        }
        if !record.source_snapshot.still_matches(&record.source())? {
            return Err(changed());
        }
        if record.stage == UndoStage::Ready {
            if cancel.load(Ordering::Acquire) {
                return Err(interrupted());
            }
            self.restore_replacement(record)?;
        }
        if record.stage != UndoStage::ReplacementRestored {
            return Err(invalid_data("replacement undo is in an unsupported state"));
        }
        let backup = record
            .backup()
            .ok_or_else(|| invalid_data("replacement undo has no backup"))?;
        self.stage_cleanup(record, &backup)?;
        progress(CopyActivity::Finishing);
        self.finish_cleanup(record)
    }

    fn undo_move(
        &self,
        fs: &impl FileSystem,
        record: &mut UndoRecord,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        if record.stage == UndoStage::CleanupStaged {
            return self.finish_cleanup(record);
        }
        if record.stage == UndoStage::SourceRestored {
            self.stage_cleanup(record, &record.destination())?;
            return self.finish_cleanup(record);
        }
        self.require_vacant_source(record)?;
        let destination = record.destination();
        if record.stage == UndoStage::Ready
            && !record
                .destination_snapshot
                .still_matches_after_rename(&destination)?
        {
            return Err(changed());
        }
        let copy_required = self.copy_back_required(fs, record, &destination, cancel)?;
        if copy_required {
            self.restore_source_by_copy(
                fs,
                record,
                &destination,
                &record.destination_snapshot.clone(),
                cancel,
                progress,
            )?;
            if record.stage != UndoStage::SourceRestored {
                return Err(interrupted());
            }
            self.stage_cleanup(record, &destination)?;
            self.finish_cleanup(record)
        } else {
            self.restore_source_by_rename(
                record,
                &destination,
                &record.destination_snapshot.clone(),
            )
        }
    }

    fn undo_move_replace(
        &self,
        fs: &impl FileSystem,
        record: &mut UndoRecord,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        if record.stage == UndoStage::CleanupStaged {
            return self.finish_cleanup(record);
        }
        if record.stage == UndoStage::Ready {
            self.require_vacant_source(record)?;
            if cancel.load(Ordering::Acquire) {
                return Err(interrupted());
            }
            self.restore_replacement(record)?;
        }
        if record.stage == UndoStage::SourceRestored {
            let backup = record
                .backup()
                .ok_or_else(|| invalid_data("move replacement undo has no backup"))?;
            self.stage_cleanup(record, &backup)?;
            return self.finish_cleanup(record);
        }
        if !matches!(
            record.stage,
            UndoStage::ReplacementRestored
                | UndoStage::SourceCopying
                | UndoStage::SourceCopyComplete
        ) {
            return Err(invalid_data(
                "move replacement undo is in an unsupported state",
            ));
        }
        self.require_vacant_source(record)?;
        let backup = record
            .backup()
            .ok_or_else(|| invalid_data("move replacement undo has no backup"))?;
        let copy_required = self.copy_back_required(fs, record, &backup, cancel)?;
        if copy_required {
            self.restore_source_by_copy(
                fs,
                record,
                &backup,
                &record.destination_snapshot.clone(),
                cancel,
                progress,
            )?;
            if record.stage != UndoStage::SourceRestored {
                return Err(interrupted());
            }
            self.stage_cleanup(record, &backup)?;
            self.finish_cleanup(record)
        } else {
            self.restore_source_by_rename(record, &backup, &record.destination_snapshot.clone())
        }
    }

    fn restore_replacement(&self, record: &mut UndoRecord) -> io::Result<()> {
        let backup = record
            .backup()
            .ok_or_else(|| invalid_data("replacement undo has no backup"))?;
        let old = record
            .replaced_snapshot
            .as_ref()
            .ok_or_else(|| invalid_data("replacement undo has no prior-item evidence"))?;
        if !record
            .destination_snapshot
            .still_matches_after_rename(&record.destination())?
            || !old.still_matches_after_rename(&backup)?
        {
            return Err(changed());
        }
        rename_exchange(&backup, &record.destination())?;
        sync_directory(
            record
                .destination()
                .parent()
                .ok_or_else(|| invalid_data("replacement destination has no parent"))?,
        )?;
        if !old.still_matches_after_rename(&record.destination())?
            || !record
                .destination_snapshot
                .still_matches_after_rename(&backup)?
        {
            return Err(invalid_data(
                "replacement identities changed during Undo exchange",
            ));
        }
        record.stage = UndoStage::ReplacementRestored;
        self.persist(record, false)
    }

    fn restore_source_by_rename(
        &self,
        record: &mut UndoRecord,
        from: &Path,
        expected: &TreeSnapshot,
    ) -> io::Result<()> {
        self.require_vacant_source(record)?;
        if !expected.still_matches_after_rename(from)? {
            return Err(changed());
        }
        rename_noreplace(from, &record.source())?;
        sync_rename_parents(from, &record.source())?;
        if !expected.still_matches_after_rename(&record.source())? {
            return Err(invalid_data(
                "restored move identity changed during atomic rename",
            ));
        }
        self.finish(record)
    }

    fn restore_source_by_copy(
        &self,
        fs: &impl FileSystem,
        record: &mut UndoRecord,
        from: &Path,
        expected: &TreeSnapshot,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        if record.stage == UndoStage::SourceCopying {
            self.discard_partial_restore(record)?;
        }
        if record.stage == UndoStage::ReplacementRestored || record.stage == UndoStage::Ready {
            self.require_vacant_source(record)?;
            if !expected.still_matches_after_rename(from)? {
                return Err(changed());
            }
            let source_parent = record
                .source()
                .parent()
                .ok_or_else(|| invalid_data("undo source has no parent"))?
                .to_path_buf();
            file_ops::ensure_copy_capacity(fs, from, &source_parent, cancel)?;
            if cancel.load(Ordering::Acquire) {
                return Err(interrupted());
            }
            record.stage = UndoStage::SourceCopying;
            record.restore_container_identity = None;
            record.restored_snapshot = None;
            self.persist(record, false)?;
            let container = record.restore_staging();
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder.create(&container)?;
            sync_directory(&source_parent)?;
            record.restore_container_identity = Some(EntryIdentity::capture(&container)?);
            self.persist(record, false)?;
            fs.copy_cancellable(from, &record.restore_payload(), cancel, progress)?;
            let restored = TreeSnapshot::capture(&record.restore_payload())?;
            if !expected.still_matches_after_rename(from)? {
                return Err(changed());
            }
            record.restored_snapshot = Some(restored);
            record.stage = UndoStage::SourceCopyComplete;
            self.persist(record, false)?;
        }
        if record.stage != UndoStage::SourceCopyComplete {
            return Err(invalid_data("undo source copy did not complete"));
        }
        if cancel.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        self.require_vacant_source(record)?;
        if !expected.still_matches_after_rename(from)? {
            return Err(changed());
        }
        let restored = record
            .restored_snapshot
            .as_ref()
            .ok_or_else(|| invalid_data("undo source copy has no completed evidence"))?;
        if !restored.still_matches(&record.restore_payload())? {
            return Err(changed());
        }
        rename_noreplace(&record.restore_payload(), &record.source())?;
        sync_rename_parents(&record.restore_payload(), &record.source())?;
        if !restored.still_matches_after_rename(&record.source())? {
            return Err(invalid_data(
                "restored source identity changed during publication",
            ));
        }
        self.remove_restore_container(record)?;
        record.stage = UndoStage::SourceRestored;
        self.persist(record, false)
    }

    fn copy_back_required(
        &self,
        fs: &impl FileSystem,
        record: &UndoRecord,
        from: &Path,
        cancel: &AtomicBool,
    ) -> io::Result<bool> {
        self.require_source_parent(record)?;
        let parent = record
            .source()
            .parent()
            .ok_or_else(|| invalid_data("undo source has no parent"))?
            .to_path_buf();
        Ok(fs.source_device(from, cancel)? != fs.destination_space(&parent)?.device())
    }

    fn require_source_parent(&self, record: &UndoRecord) -> io::Result<()> {
        let source = record.source();
        let parent = source
            .parent()
            .ok_or_else(|| invalid_data("undo source has no parent"))?;
        let current = EntryIdentity::capture(parent)?;
        if !record.source_parent_identity.same_object(&current) {
            return Err(changed());
        }
        Ok(())
    }

    fn require_vacant_source(&self, record: &UndoRecord) -> io::Result<()> {
        self.require_source_parent(record)?;
        if entry_exists(&record.source())? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the original location is no longer available",
            ));
        }
        Ok(())
    }

    fn stage_cleanup(&self, record: &mut UndoRecord, from: &Path) -> io::Result<()> {
        if !record
            .destination_snapshot
            .still_matches_after_rename(from)?
        {
            return Err(changed());
        }
        if entry_exists(&record.cleanup())? {
            return Err(changed());
        }
        rename_noreplace(from, &record.cleanup())?;
        sync_rename_parents(from, &record.cleanup())?;
        if !record
            .destination_snapshot
            .still_matches_after_rename(&record.cleanup())?
        {
            return Err(invalid_data(
                "undo cleanup identity changed during private staging",
            ));
        }
        record.stage = UndoStage::CleanupStaged;
        self.persist(record, false)
    }

    fn finish_cleanup(&self, record: &UndoRecord) -> io::Result<()> {
        let cleanup = record.cleanup();
        if entry_exists(&cleanup)? {
            remove_bound_tree(&cleanup, &record.destination_snapshot)?;
        }
        if entry_exists(&cleanup)? {
            return Err(changed());
        }
        self.finish(record)
    }

    fn discard_partial_restore(&self, record: &mut UndoRecord) -> io::Result<()> {
        let container = record.restore_staging();
        if entry_exists(&container)? {
            let metadata = fs::symlink_metadata(&container)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                return Err(changed());
            }
            if let Some(expected) = &record.restore_container_identity {
                let current = EntryIdentity::capture(&container)?;
                if !expected.same_object(&current) {
                    return Err(changed());
                }
            }
            fs::remove_dir_all(&container)?;
            sync_directory(
                container
                    .parent()
                    .ok_or_else(|| invalid_data("undo restore container has no parent"))?,
            )?;
        }
        record.restore_container_identity = None;
        record.restored_snapshot = None;
        record.stage = match record.kind {
            UndoKind::Move => UndoStage::Ready,
            UndoKind::MoveReplace => UndoStage::ReplacementRestored,
            _ => return Err(invalid_data("copying stage belongs to a non-move Undo")),
        };
        self.persist(record, false)
    }

    fn remove_restore_container(&self, record: &UndoRecord) -> io::Result<()> {
        let container = record.restore_staging();
        if !entry_exists(&container)? {
            return Ok(());
        }
        let expected = record
            .restore_container_identity
            .as_ref()
            .ok_or_else(|| invalid_data("undo restore container has no identity"))?;
        let current = EntryIdentity::capture(&container)?;
        if !expected.same_object(&current) {
            return Err(changed());
        }
        fs::remove_dir(&container)?;
        sync_directory(
            container
                .parent()
                .ok_or_else(|| invalid_data("undo restore container has no parent"))?,
        )
    }

    fn finish(&self, record: &UndoRecord) -> io::Result<()> {
        fs::remove_file(self.record_path(&record.id))?;
        sync_directory(&self.root)
    }

    fn prune_ready(&self) -> io::Result<()> {
        let mut records = self.read_records()?;
        records.sort_by_key(|record| {
            (
                record.created_seconds,
                record.created_nanoseconds,
                record.id.clone(),
            )
        });
        let excess = records.len().saturating_sub(RETAIN_READY_RECORDS);
        for record in records.into_iter().take(excess) {
            if record.stage != UndoStage::Ready {
                continue;
            }
            if let Some(backup) = record.backup() {
                let Some(expected) = record.replaced_snapshot.as_ref() else {
                    continue;
                };
                if expected.still_matches_after_rename(&backup)? {
                    remove_bound_tree(&backup, expected)?;
                } else {
                    continue;
                }
            }
            fs::remove_file(self.record_path(&record.id))?;
            sync_directory(&self.root)?;
        }
        Ok(())
    }

    fn acquire_lock(&self) -> io::Result<File> {
        let path = self.root.join("undo.lock");
        let descriptor = match rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        ) {
            Ok(descriptor) => {
                let file = File::from(descriptor);
                file.sync_all()?;
                sync_directory(&self.root)?;
                file
            }
            Err(error)
                if io::Error::from_raw_os_error(error.raw_os_error()).kind()
                    == io::ErrorKind::AlreadyExists =>
            {
                File::from(
                    rustix::fs::open(
                        &path,
                        rustix::fs::OFlags::RDWR
                            | rustix::fs::OFlags::CLOEXEC
                            | rustix::fs::OFlags::NOFOLLOW,
                        rustix::fs::Mode::empty(),
                    )
                    .map_err(io::Error::from)?,
                )
            }
            Err(error) => return Err(io::Error::from(error)),
        };
        let metadata = descriptor.metadata()?;
        let identity = EntryIdentity::capture_file(&descriptor)?;
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.len() != 0
            || EntryIdentity::capture(&path)? != identity
        {
            return Err(invalid_data(
                "undo lock is not a private empty regular file",
            ));
        }
        rustix::fs::flock(&descriptor, rustix::fs::FlockOperation::LockExclusive)
            .map_err(io::Error::from)?;
        if EntryIdentity::capture(&path)? != identity
            || EntryIdentity::capture_file(&descriptor)? != identity
        {
            return Err(invalid_data("undo lock identity changed while waiting"));
        }
        Ok(descriptor)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn persist(&self, record: &UndoRecord, create: bool) -> io::Result<()> {
        record.validate(&record.id)?;
        let mut bytes = serde_json::to_vec(record).map_err(invalid_json)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(invalid_data("undo receipt is too large"));
        }
        let temp = self
            .root
            .join(format!(".{}.{}.tmp", record.id, Uuid::new_v4()));
        let destination = self.record_path(&record.id);
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            if create {
                rename_noreplace(&temp, &destination)?;
            } else {
                fs::rename(&temp, &destination)?;
            }
            sync_directory(&self.root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    fn read_records(&self) -> io::Result<Vec<UndoRecord>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid_data("undo entry name is not UTF-8"))?;
            if name == "undo.lock" {
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.permissions().mode() & 0o777 != 0o600
                    || metadata.len() != 0
                {
                    return Err(invalid_data(
                        "undo lock is not a private empty regular file",
                    ));
                }
                continue;
            }
            if is_temp_name(name) {
                continue;
            }
            if !name.ends_with(".json") {
                return Err(invalid_data("unexpected file in undo journal"));
            }
            paths.push(path);
            if paths.len() > MAX_RECORDS {
                return Err(invalid_data("too many retained undo receipts"));
            }
        }
        paths.sort();
        paths
            .iter()
            .map(|path| self.read_record_path(path))
            .collect()
    }

    fn read_record_path(&self, path: &Path) -> io::Result<UndoRecord> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.len() > MAX_RECORD_BYTES
        {
            return Err(invalid_data(
                "undo receipt is not a bounded private regular file",
            ));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(path)?
            .take(MAX_RECORD_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(invalid_data("undo receipt is too large"));
        }
        let record: UndoRecord = serde_json::from_slice(&bytes).map_err(invalid_json)?;
        let id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid_data("undo receipt has an invalid identity"))?;
        record.validate(id)?;
        Ok(record)
    }

    fn remove_abandoned_temps(&self) -> io::Result<()> {
        let mut removed = false;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !is_temp_name(name) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.permissions().mode() & 0o777 != 0o600
                || metadata.len() > MAX_RECORD_BYTES
            {
                return Err(invalid_data(
                    "abandoned undo temporary is not a bounded private regular file",
                ));
            }
            fs::remove_file(entry.path())?;
            removed = true;
        }
        if removed {
            sync_directory(&self.root)?;
        }
        Ok(())
    }
}

fn undo_label(record: &UndoRecord) -> String {
    let name = match record.kind {
        UndoKind::Move | UndoKind::MoveReplace => record
            .source()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        UndoKind::Copy | UndoKind::Replace => record
            .destination()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
    }
    .map(|name| bounded_display_name(&name))
    .unwrap_or_else(|| "item".to_string());
    let verb = match record.kind {
        UndoKind::Copy => "Undo Copy",
        UndoKind::Move => "Undo Move",
        UndoKind::Replace => "Undo Replace",
        UndoKind::MoveReplace => "Undo Move and Replace",
    };
    format!("{verb} “{name}”")
}

fn bounded_display_name(name: &str) -> String {
    let mut output = String::new();
    let mut truncated = false;
    for (index, character) in name.chars().enumerate() {
        if index == 120 {
            truncated = true;
            break;
        }
        output.push(if character.is_control() {
            '\u{fffd}'
        } else {
            character
        });
    }
    if truncated {
        output.push('…');
    }
    output
}

fn remove_bound_tree(path: &Path, expected: &TreeSnapshot) -> io::Result<()> {
    if !expected.same_root_object(path)? {
        return Err(changed());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        if !expected.still_matches_after_rename(path)? {
            return Err(changed());
        }
        fs::remove_file(path)?;
    }
    sync_directory(
        path.parent()
            .ok_or_else(|| invalid_data("undo cleanup has no parent"))?,
    )
}

fn is_temp_name(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((record, nonce)) = body.split_once('.') else {
        return false;
    };
    Uuid::parse_str(record).is_ok() && Uuid::parse_str(nonce).is_ok()
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn sync_rename_parents(source: &Path, destination: &Path) -> io::Result<()> {
    let source_parent = source
        .parent()
        .ok_or_else(|| invalid_data("undo rename source has no parent"))?;
    let destination_parent = destination
        .parent()
        .ok_or_else(|| invalid_data("undo rename destination has no parent"))?;
    sync_directory(source_parent)?;
    if destination_parent != source_parent {
        sync_directory(destination_parent)?;
    }
    Ok(())
}

fn entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_exchange(left: &Path, right: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        left,
        rustix::fs::CWD,
        right,
        rustix::fs::RenameFlags::EXCHANGE,
    )
    .map_err(io::Error::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_exchange(_left: &Path, _right: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic replacement exchange is unavailable on this platform",
    ))
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "the item changed after the operation and cannot be undone safely",
    )
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "undo cancelled")
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_ops::{RealFileSystem, VolumeSpace};
    use crate::operation_journal::Journal;
    use std::sync::atomic::AtomicBool;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rmac-files-undo-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct CrossCopyFileSystem {
        cancel_after_copy: bool,
    }

    impl FileSystem for CrossCopyFileSystem {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            fs::create_dir(path)
        }

        fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
            rename_noreplace(source, destination)
        }

        fn copy(&self, source: &Path, destination: &Path) -> io::Result<()> {
            crate::copy_item(source, destination)
        }

        fn copy_cancellable(
            &self,
            source: &Path,
            destination: &Path,
            cancel: &AtomicBool,
            progress: &mut dyn FnMut(CopyActivity),
        ) -> io::Result<()> {
            crate::copy_item_cancellable(source, destination, cancel, progress)?;
            if self.cancel_after_copy {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            let metadata = fs::symlink_metadata(path)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            }
        }

        fn source_device(&self, _path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
            if cancel.load(Ordering::Acquire) {
                return Err(interrupted());
            }
            Ok(2)
        }

        fn destination_space(&self, _parent: &Path) -> io::Result<VolumeSpace> {
            Ok(VolumeSpace::test_value(1))
        }
    }

    fn complete_copy(journal: &Journal, source: &Path, destination: &Path) {
        let mut ticket = journal.prepare_copy(source, destination).unwrap();
        crate::copy_item(source, &ticket.staging_destination()).unwrap();
        ticket.mark_destination_complete().unwrap();
        ticket.publish_copy().unwrap();
        ticket.commit().unwrap();
    }

    fn complete_cross_volume_move(journal: &Journal, source: &Path, destination: &Path) {
        let mut ticket = journal.prepare_move(source, destination).unwrap();
        crate::copy_item(source, &ticket.staging_destination()).unwrap();
        ticket.mark_destination_complete().unwrap();
        if fs::symlink_metadata(source).unwrap().is_dir() {
            fs::remove_dir_all(source).unwrap();
        } else {
            fs::remove_file(source).unwrap();
        }
        ticket.mark_source_removed().unwrap();
        ticket.publish().unwrap();
        ticket.commit().unwrap();
    }

    #[test]
    fn committed_copy_creates_a_labeled_durable_undo_receipt() {
        let root = TestDirectory::new("copy-receipt");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"bytes").unwrap();
        let journal_root = root.0.join("journal");
        let journal = Journal::open(journal_root.clone()).unwrap();
        complete_copy(&journal, &source, &destination);

        assert_eq!(journal.undo_store().count().unwrap(), 1);
        assert_eq!(
            journal.undo_store().latest().unwrap().unwrap().label,
            "Undo Copy “destination”"
        );
        assert_eq!(
            Journal::open(journal_root)
                .unwrap()
                .undo_store()
                .count()
                .unwrap(),
            1
        );
    }

    #[test]
    fn copy_undo_removes_only_the_exact_copy_when_the_source_still_matches() {
        let root = TestDirectory::new("copy");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"bytes").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        complete_copy(&journal, &source, &destination);

        let outcome = journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .unwrap();

        assert_eq!(outcome.label, "Undo Copy “destination”");
        assert_eq!(fs::read(&source).unwrap(), b"bytes");
        assert!(!destination.exists());
        assert_eq!(journal.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn changed_copy_destination_is_never_removed_by_undo() {
        let root = TestDirectory::new("copy-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"bytes").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        complete_copy(&journal, &source, &destination);
        fs::remove_file(&destination).unwrap();
        fs::write(&destination, b"replacement").unwrap();

        let error = journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(&destination).unwrap(), b"replacement");
        assert_eq!(journal.undo_store().count().unwrap(), 1);
    }

    #[test]
    fn changed_copy_source_prevents_undo_from_deleting_the_only_proven_copy() {
        let root = TestDirectory::new("copy-source-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"bytes").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        complete_copy(&journal, &source, &destination);
        fs::remove_file(&source).unwrap();

        let error = journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(&destination).unwrap(), b"bytes");
        assert_eq!(journal.undo_store().count().unwrap(), 1);
    }

    #[test]
    fn copy_replacement_undo_atomically_restores_the_previous_item() {
        let root = TestDirectory::new("replace");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_replace(&source, &destination).unwrap();
        crate::copy_item(&source, &ticket.staging_destination()).unwrap();
        ticket.mark_destination_complete().unwrap();
        ticket.replace_copy().unwrap();
        ticket.commit().unwrap();

        journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert_eq!(journal.undo_store().count().unwrap(), 0);
        assert_eq!(
            fs::read_dir(&root.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(".rmac-"))
                .count(),
            0
        );
    }

    #[test]
    fn replacement_undo_infers_an_exchange_before_stage_persistence() {
        let root = TestDirectory::new("replace-exchange-crash");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();
        let journal_root = root.0.join("journal");
        let journal = Journal::open(journal_root.clone()).unwrap();
        let mut ticket = journal.prepare_replace(&source, &destination).unwrap();
        crate::copy_item(&source, &ticket.staging_destination()).unwrap();
        ticket.mark_destination_complete().unwrap();
        ticket.replace_copy().unwrap();
        ticket.commit().unwrap();
        let record = journal.undo_store().latest_record().unwrap().unwrap();
        let backup = record.backup().unwrap();
        rename_exchange(&backup, &destination).unwrap();
        sync_directory(&root.0).unwrap();
        drop(journal);
        let reopened = Journal::open(journal_root).unwrap();

        reopened
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert!(!backup.exists());
        assert_eq!(reopened.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn changed_replacement_backup_is_never_exchanged_or_removed_by_undo() {
        let root = TestDirectory::new("replace-backup-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_replace(&source, &destination).unwrap();
        crate::copy_item(&source, &ticket.staging_destination()).unwrap();
        ticket.mark_destination_complete().unwrap();
        ticket.replace_copy().unwrap();
        ticket.commit().unwrap();
        let record = journal.undo_store().latest_record().unwrap().unwrap();
        let backup = record.backup().unwrap();
        fs::write(&backup, b"changed old item").unwrap();

        let error = journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert_eq!(fs::read(&backup).unwrap(), b"changed old item");
        assert_eq!(journal.undo_store().count().unwrap(), 1);
    }

    #[test]
    fn same_volume_move_undo_restores_the_exact_source_without_copying() {
        let root = TestDirectory::new("move");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"moved").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        ticket.stage_source_for_move().unwrap();
        ticket.publish().unwrap();
        ticket.commit().unwrap();

        journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"moved");
        assert!(!destination.exists());
        assert_eq!(journal.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn move_undo_never_replaces_a_racing_original_location() {
        let root = TestDirectory::new("move-source-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"moved").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        ticket.stage_source_for_move().unwrap();
        ticket.publish().unwrap();
        ticket.commit().unwrap();
        fs::write(&source, b"racing item").unwrap();

        let error = journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&source).unwrap(), b"racing item");
        assert_eq!(fs::read(&destination).unwrap(), b"moved");
        assert_eq!(journal.undo_store().count().unwrap(), 1);
    }

    #[test]
    fn same_volume_move_replacement_undo_restores_both_exact_items() {
        let root = TestDirectory::new("move-replace");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move_replace(&source, &destination).unwrap();
        ticket.stage_source_for_move_replace().unwrap();
        ticket.replace_move().unwrap();
        ticket.commit().unwrap();

        journal
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert_eq!(journal.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn cross_volume_move_undo_resumes_after_cancellation_at_a_durable_boundary() {
        let root = TestDirectory::new("cross-move");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"moved").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        complete_cross_volume_move(&journal, &source, &destination);
        let cancel = AtomicBool::new(false);

        let error = journal
            .undo_store()
            .execute_latest(
                &CrossCopyFileSystem {
                    cancel_after_copy: true,
                },
                &cancel,
                &mut |_| {},
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"moved");
        cancel.store(false, Ordering::Release);

        journal
            .undo_store()
            .execute_latest(
                &CrossCopyFileSystem {
                    cancel_after_copy: false,
                },
                &cancel,
                &mut |_| {},
            )
            .unwrap()
            .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"moved");
        assert!(!destination.exists());
        assert_eq!(journal.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn cross_volume_move_replacement_undo_restores_both_items() {
        let root = TestDirectory::new("cross-move-replace");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move_replace(&source, &destination).unwrap();
        crate::copy_item(&source, &ticket.staging_destination()).unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::remove_file(&source).unwrap();
        ticket.mark_source_removed().unwrap();
        ticket.replace_move().unwrap();
        ticket.commit().unwrap();

        journal
            .undo_store()
            .execute_latest(
                &CrossCopyFileSystem {
                    cancel_after_copy: false,
                },
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap()
            .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert_eq!(journal.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn restart_infers_copy_cleanup_staged_before_receipt_persistence() {
        let root = TestDirectory::new("copy-cleanup-crash");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"bytes").unwrap();
        let journal_root = root.0.join("journal");
        let journal = Journal::open(journal_root.clone()).unwrap();
        complete_copy(&journal, &source, &destination);
        let record = journal.undo_store().latest_record().unwrap().unwrap();
        rename_noreplace(&destination, &record.cleanup()).unwrap();
        sync_rename_parents(&destination, &record.cleanup()).unwrap();
        drop(journal);
        let reopened = Journal::open(journal_root).unwrap();

        reopened
            .undo_store()
            .execute_latest(&RealFileSystem, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .unwrap();

        assert!(source.exists());
        assert!(!destination.exists());
        assert!(!record.cleanup().exists());
        assert_eq!(reopened.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn ready_receipts_are_pruned_to_the_bounded_history() {
        let root = TestDirectory::new("prune");
        let journal = Journal::open(root.0.join("journal")).unwrap();
        for index in 0..=RETAIN_READY_RECORDS {
            let source = root.0.join(format!("source-{index}"));
            let destination = root.0.join(format!("destination-{index}"));
            fs::write(&source, index.to_string()).unwrap();
            complete_copy(&journal, &source, &destination);
        }

        assert_eq!(journal.undo_store().count().unwrap(), RETAIN_READY_RECORDS);
        assert!(root.0.join("destination-0").exists());
    }
}
