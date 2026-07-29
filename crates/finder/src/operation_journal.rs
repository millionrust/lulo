use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const JOURNAL_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_RECORDS: usize = 512;
const MAX_MANIFEST_ENTRIES: u64 = 1_000_000;
const MAX_MANIFEST_DEPTH: usize = 256;
const MAX_MANIFEST_PATH_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum JournalOperation {
    Copy,
    #[default]
    Move,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransferStage {
    Prepared,
    DestinationComplete,
    SourceRemoved,
    Published,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResolutionIntent {
    PreserveCopy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct EntryIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl EntryIdentity {
    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(Self::from_metadata(&metadata))
    }

    pub(crate) fn capture_file(file: &File) -> io::Result<Self> {
        Ok(Self::from_metadata(&file.metadata()?))
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }

    /// Renaming an entry can legitimately update ctime. The durable content
    /// identity must otherwise remain the same across publication.
    pub(crate) fn same_entry_after_rename(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.mode == other.mode
            && self.size == other.size
            && self.modified_seconds == other.modified_seconds
            && self.modified_nanoseconds == other.modified_nanoseconds
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct TreeManifest {
    entries: u64,
    path_bytes: u64,
    sha256: [u8; 32],
    #[serde(default)]
    rename_stable_sha256: [u8; 32],
}

impl TreeManifest {
    pub(crate) fn capture(root: &Path) -> io::Result<Self> {
        Self::capture_inner(root, None)
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn capture_cancellable(root: &Path, cancel: &AtomicBool) -> io::Result<Self> {
        Self::capture_inner(root, Some(cancel))
    }

    fn capture_inner(root: &Path, cancel: Option<&AtomicBool>) -> io::Result<Self> {
        let mut builder = ManifestBuilder {
            exact: Sha256::new(),
            rename_stable: Sha256::new(),
            entries: 0,
            path_bytes: 0,
        };
        builder.update(b"rmac-tree-manifest-v1\0");
        capture_manifest_entry(root, Path::new(""), 0, &mut builder, cancel)?;
        Ok(Self {
            entries: builder.entries,
            path_bytes: builder.path_bytes,
            sha256: builder.exact.finalize().into(),
            rename_stable_sha256: builder.rename_stable.finalize().into(),
        })
    }

    pub(crate) fn same_after_root_rename(&self, other: &Self) -> bool {
        self.entries == other.entries
            && self.path_bytes == other.path_bytes
            && self.rename_stable_sha256 == other.rename_stable_sha256
    }
}

struct ManifestBuilder {
    exact: Sha256,
    rename_stable: Sha256,
    entries: u64,
    path_bytes: u64,
}

impl ManifestBuilder {
    fn update(&mut self, bytes: &[u8]) {
        self.exact.update(bytes);
        self.rename_stable.update(bytes);
    }

    fn hash_u64(&mut self, value: u64) {
        self.update(&value.to_le_bytes());
    }

    fn hash_identity(&mut self, identity: &EntryIdentity, is_root: bool) {
        hash_identity(&mut self.exact, identity, true);
        hash_identity(&mut self.rename_stable, identity, !is_root);
    }
}

fn capture_manifest_entry(
    path: &Path,
    relative: &Path,
    depth: usize,
    builder: &mut ManifestBuilder,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "tree manifest cancelled",
        ));
    }
    if depth > MAX_MANIFEST_DEPTH {
        return Err(invalid_data(
            "source folder nesting exceeds the manifest safety limit",
        ));
    }
    builder.entries = builder
        .entries
        .checked_add(1)
        .filter(|entries| *entries <= MAX_MANIFEST_ENTRIES)
        .ok_or_else(|| invalid_data("source contains too many manifest entries"))?;
    let relative_bytes = relative.as_os_str().as_bytes();
    builder.path_bytes = builder
        .path_bytes
        .checked_add(relative_bytes.len() as u64)
        .filter(|bytes| *bytes <= MAX_MANIFEST_PATH_BYTES)
        .ok_or_else(|| invalid_data("source manifest paths exceed the safety limit"))?;

    let before = EntryIdentity::capture(path)?;
    builder.hash_u64(relative_bytes.len() as u64);
    builder.update(relative_bytes);
    builder.hash_identity(&before, depth == 0);

    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path)?;
        let target = target.as_os_str().as_bytes();
        builder.path_bytes = builder
            .path_bytes
            .checked_add(target.len() as u64)
            .filter(|bytes| *bytes <= MAX_MANIFEST_PATH_BYTES)
            .ok_or_else(|| invalid_data("source manifest paths exceed the safety limit"))?;
        builder.hash_u64(target.len() as u64);
        builder.update(target);
        if EntryIdentity::capture(path)? != before {
            return Err(manifest_changed());
        }
        return Ok(());
    }

    builder.hash_u64(u64::MAX);
    if metadata.is_dir() {
        let mut names = Vec::<OsString>::new();
        let mut directory_name_bytes = 0u64;
        for entry in fs::read_dir(path)? {
            let name = entry?.file_name();
            if names.len() >= MAX_MANIFEST_ENTRIES as usize {
                return Err(invalid_data(
                    "source directory contains too many manifest entries",
                ));
            }
            directory_name_bytes = directory_name_bytes
                .checked_add(name.as_bytes().len() as u64)
                .filter(|bytes| *bytes <= MAX_MANIFEST_PATH_BYTES)
                .ok_or_else(|| invalid_data("source directory names exceed the safety limit"))?;
            names.push(name);
        }
        names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        for name in names {
            capture_manifest_entry(
                &path.join(&name),
                &relative.join(&name),
                depth + 1,
                builder,
                cancel,
            )?;
        }
        if EntryIdentity::capture(path)? != before {
            return Err(manifest_changed());
        }
    } else if EntryIdentity::capture(path)? != before {
        return Err(manifest_changed());
    }
    Ok(())
}

fn hash_identity(hasher: &mut Sha256, identity: &EntryIdentity, include_changed_time: bool) {
    hash_u64(hasher, identity.device);
    hash_u64(hasher, identity.inode);
    hasher.update(identity.mode.to_le_bytes());
    hash_u64(hasher, identity.size);
    hasher.update(identity.modified_seconds.to_le_bytes());
    hasher.update(identity.modified_nanoseconds.to_le_bytes());
    if include_changed_time {
        hasher.update(identity.changed_seconds.to_le_bytes());
        hasher.update(identity.changed_nanoseconds.to_le_bytes());
    }
}

fn hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

fn manifest_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "tree changed while its transfer manifest was being captured",
    )
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct TransferRecord {
    version: u32,
    id: String,
    /// Version-1 records created before copy journaling omitted this field and
    /// are moves. Keep that durable compatibility instead of forcing users to
    /// discard an unfinished operation during upgrade.
    #[serde(default)]
    operation: JournalOperation,
    stage: TransferStage,
    source_path_bytes: Vec<u8>,
    destination_path_bytes: Vec<u8>,
    staging_path_bytes: Vec<u8>,
    source_identity: EntryIdentity,
    #[serde(default)]
    source_manifest: Option<TreeManifest>,
    destination_identity: Option<EntryIdentity>,
    #[serde(default)]
    destination_manifest: Option<TreeManifest>,
    #[serde(default)]
    resolution_intent: Option<ResolutionIntent>,
}

impl TransferRecord {
    fn source(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.source_path_bytes.clone()))
    }

    fn destination(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.destination_path_bytes.clone()))
    }

    fn staging_destination(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.staging_path_bytes.clone()))
    }

    fn source_still_matches(&self) -> io::Result<bool> {
        if let Some(expected) = &self.source_manifest {
            return match TreeManifest::capture(&self.source()) {
                Ok(current) => Ok(&current == expected),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::WouldBlock
                    ) =>
                {
                    Ok(false)
                }
                Err(error) => Err(error),
            };
        }
        match EntryIdentity::capture(&self.source()) {
            Ok(identity) => Ok(identity == self.source_identity),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn destination_path_matches(&self, path: &Path, published: bool) -> io::Result<bool> {
        let Some(expected_identity) = &self.destination_identity else {
            return Ok(false);
        };
        let current_identity = match EntryIdentity::capture(path) {
            Ok(identity) => identity,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let identity_matches = if published {
            expected_identity.same_entry_after_rename(&current_identity)
        } else {
            expected_identity == &current_identity
        };
        if !identity_matches {
            return Ok(false);
        }
        let Some(expected_manifest) = &self.destination_manifest else {
            return Ok(true);
        };
        let current_manifest = match TreeManifest::capture(path) {
            Ok(manifest) => manifest,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::WouldBlock
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        Ok(if published {
            expected_manifest.same_after_root_rename(&current_manifest)
        } else {
            expected_manifest == &current_manifest
        })
    }

    fn validate(&self, expected_id: &str) -> io::Result<()> {
        if self.version != JOURNAL_VERSION {
            return Err(invalid_data("unsupported file-operation journal version"));
        }
        if self.id != expected_id || Uuid::parse_str(&self.id).is_err() {
            return Err(invalid_data("file-operation journal identity mismatch"));
        }
        let source = self.source();
        let destination = self.destination();
        let staging = self.staging_destination();
        let expected_staging_name = format!(".rmac-transfer-{}", self.id);
        if !source.is_absolute() || !destination.is_absolute() || !staging.is_absolute() {
            return Err(invalid_data(
                "file-operation journal paths must be absolute",
            ));
        }
        if source == destination
            || source == staging
            || destination == staging
            || destination.parent() != staging.parent()
            || staging.file_name().and_then(|name| name.to_str())
                != Some(expected_staging_name.as_str())
        {
            return Err(invalid_data(
                "file-operation journal path relationship is invalid",
            ));
        }
        if self.stage == TransferStage::Prepared
            && (self.destination_identity.is_some() || self.destination_manifest.is_some())
        {
            return Err(invalid_data(
                "prepared journal unexpectedly has destination evidence",
            ));
        }
        if self.stage != TransferStage::Prepared && self.destination_identity.is_none() {
            return Err(invalid_data(
                "advanced journal is missing its destination identity",
            ));
        }
        if self.operation == JournalOperation::Copy && self.stage == TransferStage::SourceRemoved {
            return Err(invalid_data(
                "copy journal cannot contain a source-removed stage",
            ));
        }
        if self.operation == JournalOperation::Copy && self.source_manifest.is_none() {
            return Err(invalid_data("copy journal is missing its source manifest"));
        }
        if self.stage != TransferStage::Prepared
            && self.source_manifest.is_some()
            && self.destination_manifest.is_none()
        {
            return Err(invalid_data(
                "advanced journal is missing its destination manifest",
            ));
        }
        if self.resolution_intent == Some(ResolutionIntent::PreserveCopy)
            && !matches!(
                self.stage,
                TransferStage::DestinationComplete | TransferStage::Published
            )
        {
            return Err(invalid_data(
                "preserve-copy intent has an invalid journal stage",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Journal {
    root: PathBuf,
}

struct RecordLock {
    file: File,
    path: PathBuf,
    identity: EntryIdentity,
}

impl RecordLock {
    fn remove_path(&self) -> io::Result<()> {
        if EntryIdentity::capture(&self.path)? != self.identity
            || EntryIdentity::capture_file(&self.file)? != self.identity
        {
            return Err(invalid_data("file-operation lock identity changed"));
        }
        fs::remove_file(&self.path)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecoveryReport {
    pub(crate) finalized: usize,
    pub(crate) pending: usize,
    pub(crate) active: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    PreserveCopy {
        complete: bool,
        suggested_name: String,
    },
    KeepExistingItems,
}

impl fmt::Debug for RecoveryAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreserveCopy { complete, .. } => formatter
                .debug_struct("PreserveCopy")
                .field("complete", complete)
                .finish_non_exhaustive(),
            Self::KeepExistingItems => formatter.write_str("KeepExistingItems"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct RecoveryReview {
    record: TransferRecord,
    journal_identity: EntryIdentity,
    source_snapshot: Option<EntryIdentity>,
    source_manifest_matches: bool,
    staging_snapshot: Option<EntryIdentity>,
    staging_manifest_matches: bool,
    destination_snapshot: Option<EntryIdentity>,
    preserve_destination: Option<PathBuf>,
    pub(crate) action: RecoveryAction,
}

impl fmt::Debug for RecoveryReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryReview")
            .field("operation", &self.record.operation)
            .field("stage", &self.record.stage)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ResolutionOutcome {
    PreservedCopy { complete: bool, name: String },
    KeptExistingItems,
}

impl fmt::Debug for ResolutionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreservedCopy { complete, .. } => formatter
                .debug_struct("PreservedCopy")
                .field("complete", complete)
                .finish_non_exhaustive(),
            Self::KeptExistingItems => formatter.write_str("KeptExistingItems"),
        }
    }
}

impl Journal {
    pub(crate) fn open_default() -> io::Result<Self> {
        let root = match std::env::var_os("XDG_STATE_HOME") {
            Some(path) if !path.is_empty() => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "XDG_STATE_HOME must be absolute",
                    ));
                }
                path.join("rmac-files").join("operations")
            }
            _ => {
                let home = std::env::var_os("HOME").ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "HOME is unavailable for file-operation recovery",
                    )
                })?;
                let home = PathBuf::from(home);
                if !home.is_absolute() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "HOME must be absolute",
                    ));
                }
                home.join(".local")
                    .join("state")
                    .join("rmac-files")
                    .join("operations")
            }
        };
        Self::open(root)
    }

    pub(crate) fn open(root: PathBuf) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file-operation journal root must be absolute",
            ));
        }
        fs::create_dir_all(&root)?;
        let metadata = fs::symlink_metadata(&root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-operation journal root is not a real directory",
            ));
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let journal = Self { root };
        journal.remove_abandoned_temps()?;
        journal.remove_orphan_locks()?;
        Ok(journal)
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> io::Result<usize> {
        Ok(self.read_records()?.len())
    }

    /// Finish only states whose recorded identities prove that source removal
    /// already happened and that the complete staged copy is still ours. This
    /// never removes a source, a partial copy, or a conflicting final path.
    pub(crate) fn recover_unambiguous(&self) -> io::Result<RecoveryReport> {
        let records = self.read_records()?;
        let mut report = RecoveryReport::default();
        for record in records {
            let Some(lock) = self.try_lock_record(&record.id)? else {
                report.active += 1;
                continue;
            };
            let path = self.record_path(&record.id);
            let mut ticket = TransferTicket {
                journal: self.clone(),
                path,
                record,
                lock,
            };
            if self.recover_ticket(&mut ticket)? {
                report.finalized += 1;
            } else {
                report.pending += 1;
            }
        }
        Ok(report)
    }

    fn recover_ticket(&self, ticket: &mut TransferTicket) -> io::Result<bool> {
        let source = capture_optional(&ticket.record.source())?;
        let staging = capture_optional(&ticket.record.staging_destination())?;
        let destination = capture_optional(&ticket.record.destination())?;
        let source_missing = source.is_none();
        let staging_matches = staging.is_some()
            && ticket
                .record
                .destination_path_matches(&ticket.record.staging_destination(), false)?;
        let destination_matches = destination.is_some()
            && ticket
                .record
                .destination_path_matches(&ticket.record.destination(), true)?;

        if ticket.record.resolution_intent == Some(ResolutionIntent::PreserveCopy) {
            return match ticket.record.stage {
                TransferStage::DestinationComplete if staging_matches && destination.is_none() => {
                    ticket.publish_preserved()?;
                    self.finish_recovered_ticket(ticket)
                }
                TransferStage::DestinationComplete if staging.is_none() && destination_matches => {
                    ticket.record.destination_identity = destination;
                    ticket.record.stage = TransferStage::Published;
                    self.persist(&ticket.path, &ticket.record, false)?;
                    self.finish_recovered_ticket(ticket)
                }
                TransferStage::Published if destination_matches => {
                    self.finish_recovered_ticket(ticket)
                }
                _ => Ok(false),
            };
        }

        match ticket.record.operation {
            JournalOperation::Move => match ticket.record.stage {
                TransferStage::Prepared => Ok(false),
                TransferStage::DestinationComplete
                    if source_missing && staging_matches && destination.is_none() =>
                {
                    ticket.mark_source_removed()?;
                    self.publish_recovered_move(ticket)
                }
                TransferStage::SourceRemoved
                    if source_missing && staging_matches && destination.is_none() =>
                {
                    self.publish_recovered_move(ticket)
                }
                TransferStage::SourceRemoved
                    if source_missing && staging.is_none() && destination_matches =>
                {
                    ticket.record.destination_identity = destination;
                    ticket.record.stage = TransferStage::Published;
                    self.persist(&ticket.path, &ticket.record, false)?;
                    self.finish_recovered_ticket(ticket)
                }
                TransferStage::Published if source_missing && destination_matches => {
                    self.finish_recovered_ticket(ticket)
                }
                _ => Ok(false),
            },
            JournalOperation::Copy => {
                let source_matches = ticket.source_still_matches()?;
                match ticket.record.stage {
                    TransferStage::Prepared => Ok(false),
                    TransferStage::DestinationComplete
                        if source_matches && staging_matches && destination.is_none() =>
                    {
                        self.publish_recovered_copy(ticket)
                    }
                    TransferStage::DestinationComplete
                        if staging.is_none() && destination_matches =>
                    {
                        ticket.record.destination_identity = destination;
                        ticket.record.stage = TransferStage::Published;
                        self.persist(&ticket.path, &ticket.record, false)?;
                        self.finish_recovered_ticket(ticket)
                    }
                    TransferStage::Published if destination_matches => {
                        self.finish_recovered_ticket(ticket)
                    }
                    TransferStage::SourceRemoved => Err(invalid_data(
                        "copy journal reached an impossible source-removed stage",
                    )),
                    _ => Ok(false),
                }
            }
        }
    }

    pub(crate) fn review_pending(&self) -> io::Result<Vec<RecoveryReview>> {
        let mut reviews = Vec::new();
        for record in self.read_records()? {
            let Some(_lock) = self.try_lock_record(&record.id)? else {
                continue;
            };
            let journal_path = self.record_path(&record.id);
            let journal_identity = EntryIdentity::capture(&journal_path)?;
            let source_snapshot = capture_optional(&record.source())?;
            let staging_snapshot = capture_optional(&record.staging_destination())?;
            let destination_snapshot = capture_optional(&record.destination())?;
            let source_matches =
                record.operation != JournalOperation::Copy || record.source_still_matches()?;
            let staging_matches = staging_snapshot.is_some()
                && record.destination_path_matches(&record.staging_destination(), false)?;
            let (action, preserve_destination) = if staging_snapshot.is_some() {
                let complete =
                    staging_matches && record.stage != TransferStage::Prepared && source_matches;
                let candidate = available_recovery_destination(&record.destination())?;
                let suggested_name = candidate
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Recovered item".to_string());
                (
                    RecoveryAction::PreserveCopy {
                        complete,
                        suggested_name,
                    },
                    Some(candidate),
                )
            } else {
                (RecoveryAction::KeepExistingItems, None)
            };
            reviews.push(RecoveryReview {
                record,
                journal_identity,
                source_snapshot,
                source_manifest_matches: source_matches,
                staging_snapshot,
                staging_manifest_matches: staging_matches,
                destination_snapshot,
                preserve_destination,
                action,
            });
        }
        Ok(reviews)
    }

    pub(crate) fn resolve_review(&self, review: &RecoveryReview) -> io::Result<ResolutionOutcome> {
        let lock = self
            .try_lock_record(&review.record.id)?
            .ok_or_else(review_changed)?;
        self.validate_review(review)?;
        match &review.action {
            RecoveryAction::PreserveCopy {
                complete,
                suggested_name,
            } => {
                let candidate = review
                    .preserve_destination
                    .as_ref()
                    .ok_or_else(|| invalid_data("recovery review has no preserve destination"))?;
                if capture_optional(candidate)?.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "recovery destination is no longer available",
                    ));
                }
                let staging_identity = review
                    .staging_snapshot
                    .clone()
                    .ok_or_else(|| invalid_data("recovery copy is no longer available"))?;
                let staging_manifest = TreeManifest::capture(&review.record.staging_destination())?;
                let mut ticket = TransferTicket {
                    journal: self.clone(),
                    path: self.record_path(&review.record.id),
                    record: review.record.clone(),
                    lock,
                };
                ticket.record.destination_path_bytes = candidate.as_os_str().as_bytes().to_vec();
                ticket.record.destination_identity = Some(staging_identity);
                ticket.record.destination_manifest = Some(staging_manifest);
                ticket.record.stage = TransferStage::DestinationComplete;
                ticket.record.resolution_intent = Some(ResolutionIntent::PreserveCopy);
                self.persist(&ticket.path, &ticket.record, false)?;
                ticket.publish_preserved()?;
                ticket.commit()?;
                Ok(ResolutionOutcome::PreservedCopy {
                    complete: *complete,
                    name: suggested_name.clone(),
                })
            }
            RecoveryAction::KeepExistingItems => {
                let path = self.record_path(&review.record.id);
                self.finish_record(&path, &lock)?;
                Ok(ResolutionOutcome::KeptExistingItems)
            }
        }
    }

    fn validate_review(&self, review: &RecoveryReview) -> io::Result<()> {
        let path = self.record_path(&review.record.id);
        if EntryIdentity::capture(&path)? != review.journal_identity {
            return Err(review_changed());
        }
        let current = self
            .read_records()?
            .into_iter()
            .find(|record| record.id == review.record.id)
            .ok_or_else(review_changed)?;
        if current != review.record
            || capture_optional(&review.record.source())? != review.source_snapshot
            || (review.record.operation == JournalOperation::Copy
                && review.record.source_still_matches()? != review.source_manifest_matches)
            || capture_optional(&review.record.staging_destination())? != review.staging_snapshot
            || (review.staging_snapshot.is_some()
                && review
                    .record
                    .destination_path_matches(&review.record.staging_destination(), false)?
                    != review.staging_manifest_matches)
            || capture_optional(&review.record.destination())? != review.destination_snapshot
        {
            return Err(review_changed());
        }
        Ok(())
    }

    fn publish_recovered_move(&self, ticket: &mut TransferTicket) -> io::Result<bool> {
        match ticket.publish() {
            Ok(()) => self.finish_recovered_ticket(ticket),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn publish_recovered_copy(&self, ticket: &mut TransferTicket) -> io::Result<bool> {
        match ticket.publish_copy() {
            Ok(()) => self.finish_recovered_ticket(ticket),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn finish_recovered_ticket(&self, ticket: &TransferTicket) -> io::Result<bool> {
        if ticket.record.stage != TransferStage::Published {
            return Err(invalid_data(
                "recovered transfer did not reach published state",
            ));
        }
        self.finish_record(&ticket.path, &ticket.lock)?;
        Ok(true)
    }

    pub(crate) fn prepare_move(
        &self,
        source: &Path,
        destination: &Path,
    ) -> io::Result<TransferTicket> {
        self.prepare_transfer(JournalOperation::Move, source, destination)
    }

    pub(crate) fn prepare_copy(
        &self,
        source: &Path,
        destination: &Path,
    ) -> io::Result<TransferTicket> {
        self.prepare_transfer(JournalOperation::Copy, source, destination)
    }

    fn prepare_transfer(
        &self,
        operation: JournalOperation,
        source: &Path,
        destination: &Path,
    ) -> io::Result<TransferTicket> {
        if !source.is_absolute() || !destination.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "journaled transfers require absolute paths",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let destination_parent = destination.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "journaled transfer destination has no parent",
            )
        })?;
        let staging = destination_parent.join(format!(".rmac-transfer-{id}"));
        let source_identity = EntryIdentity::capture(source)?;
        let source_manifest = TreeManifest::capture(source)?;
        if EntryIdentity::capture(source)? != source_identity {
            return Err(manifest_changed());
        }
        let record = TransferRecord {
            version: JOURNAL_VERSION,
            id: id.clone(),
            operation,
            stage: TransferStage::Prepared,
            source_path_bytes: source.as_os_str().as_bytes().to_vec(),
            destination_path_bytes: destination.as_os_str().as_bytes().to_vec(),
            staging_path_bytes: staging.as_os_str().as_bytes().to_vec(),
            source_identity,
            source_manifest: Some(source_manifest),
            destination_identity: None,
            destination_manifest: None,
            resolution_intent: None,
        };
        record.validate(&id)?;
        let lock = self.create_active_lock(&id)?;
        let path = self.record_path(&id);
        if let Err(error) = self.persist(&path, &record, true) {
            let _ = lock.remove_path();
            let _ = sync_directory(&self.root);
            return Err(error);
        }
        Ok(TransferTicket {
            journal: self.clone(),
            path,
            record,
            lock,
        })
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn lock_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.lock"))
    }

    fn create_active_lock(&self, id: &str) -> io::Result<RecordLock> {
        let lock = self.open_record_lock(id, true)?;
        rustix::fs::flock(&lock.file, rustix::fs::FlockOperation::LockExclusive)
            .map_err(io::Error::from)?;
        lock.file.sync_all()?;
        sync_directory(&self.root)?;
        Ok(lock)
    }

    fn try_lock_record(&self, id: &str) -> io::Result<Option<RecordLock>> {
        let lock = match self.open_record_lock(id, false) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match self.open_record_lock(id, true) {
                    Ok(lock) => {
                        lock.file.sync_all()?;
                        sync_directory(&self.root)?;
                        lock
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        self.open_record_lock(id, false)?
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        match rustix::fs::flock(
            &lock.file,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .map_err(io::Error::from)
        {
            Ok(()) => Ok(Some(lock)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn open_record_lock(&self, id: &str, create: bool) -> io::Result<RecordLock> {
        if Uuid::parse_str(id).is_err() {
            return Err(invalid_data("file-operation lock identity is invalid"));
        }
        let path = self.lock_path(id);
        let mut flags =
            rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW;
        if create {
            flags |= rustix::fs::OFlags::CREATE | rustix::fs::OFlags::EXCL;
        }
        let descriptor = rustix::fs::open(
            &path,
            flags,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(io::Error::from)?;
        let file = File::from(descriptor);
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.mode() & 0o777 != 0o600 {
            return Err(invalid_data(
                "file-operation lock is not a private regular file",
            ));
        }
        let identity = EntryIdentity::from_metadata(&metadata);
        if EntryIdentity::capture(&path)? != identity {
            return Err(invalid_data("file-operation lock identity changed"));
        }
        Ok(RecordLock {
            file,
            path,
            identity,
        })
    }

    fn finish_record(&self, record_path: &Path, lock: &RecordLock) -> io::Result<()> {
        fs::remove_file(record_path)?;
        lock.remove_path()?;
        sync_directory(&self.root)
    }

    fn persist(&self, path: &Path, record: &TransferRecord, create: bool) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(record).map_err(invalid_json)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(invalid_data("file-operation journal record is too large"));
        }

        let temp = self
            .root
            .join(format!(".{}.{}.tmp", record.id, Uuid::new_v4()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            if create {
                rename_noreplace(&temp, path)?;
            } else {
                fs::rename(&temp, path)?;
            }
            sync_directory(&self.root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    fn read_records(&self) -> io::Result<Vec<TransferRecord>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid_data("journal entry name is not UTF-8"))?;
            if let Some(id) = lock_record_id(name) {
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.mode() & 0o777 != 0o600
                    || metadata.len() != 0
                {
                    return Err(invalid_data(
                        "file-operation lock is not a private empty regular file",
                    ));
                }
                if Uuid::parse_str(id).is_err() {
                    return Err(invalid_data("file-operation lock identity is invalid"));
                }
                continue;
            }
            if temp_record_id(name).is_some() {
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.mode() & 0o777 != 0o600
                    || metadata.len() > MAX_RECORD_BYTES
                {
                    return Err(invalid_data(
                        "journal temporary is not a bounded private regular file",
                    ));
                }
                continue;
            }
            if !name.ends_with(".json") {
                return Err(invalid_data("unexpected file in operation journal"));
            }
            paths.push(path);
            if paths.len() > MAX_RECORDS {
                return Err(invalid_data("too many unfinished file-operation records"));
            }
        }
        paths.sort();

        let mut records = Vec::with_capacity(paths.len());
        for path in paths {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(invalid_data("journal entry is not a regular file"));
            }
            if metadata.len() > MAX_RECORD_BYTES {
                return Err(invalid_data("file-operation journal record is too large"));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            File::open(&path)?
                .take(MAX_RECORD_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_RECORD_BYTES {
                return Err(invalid_data("file-operation journal record is too large"));
            }
            let record: TransferRecord = serde_json::from_slice(&bytes).map_err(invalid_json)?;
            let file_name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid_data("journal entry has an invalid identity"))?;
            record.validate(file_name)?;
            records.push(record);
        }
        Ok(records)
    }

    fn remove_abandoned_temps(&self) -> io::Result<()> {
        let mut removed = false;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(id) = temp_record_id(name) else {
                continue;
            };
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.mode() & 0o777 != 0o600
                || metadata.len() > MAX_RECORD_BYTES
            {
                return Err(invalid_data(
                    "abandoned journal temporary is not a bounded private regular file",
                ));
            }
            let Some(_lock) = self.try_lock_record(id)? else {
                continue;
            };
            fs::remove_file(entry.path())?;
            removed = true;
        }
        if removed {
            sync_directory(&self.root)?;
        }
        Ok(())
    }

    fn remove_orphan_locks(&self) -> io::Result<()> {
        let mut removed = false;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(id) = lock_record_id(name) else {
                continue;
            };
            match fs::symlink_metadata(self.record_path(id)) {
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let Some(lock) = self.try_lock_record(id)? else {
                continue;
            };
            lock.remove_path()?;
            removed = true;
        }
        if removed {
            sync_directory(&self.root)?;
        }
        Ok(())
    }
}

pub(crate) struct TransferTicket {
    journal: Journal,
    path: PathBuf,
    record: TransferRecord,
    lock: RecordLock,
}

impl TransferTicket {
    pub(crate) fn staging_destination(&self) -> PathBuf {
        self.record.staging_destination()
    }

    pub(crate) fn source_still_matches(&self) -> io::Result<bool> {
        self.record.source_still_matches()
    }

    pub(crate) fn mark_destination_complete(&mut self) -> io::Result<()> {
        let staging = self.record.staging_destination();
        let identity = EntryIdentity::capture(&staging)?;
        let manifest = TreeManifest::capture(&staging)?;
        if EntryIdentity::capture(&staging)? != identity {
            return Err(manifest_changed());
        }
        self.record.destination_identity = Some(identity);
        self.record.destination_manifest = Some(manifest);
        self.record.stage = TransferStage::DestinationComplete;
        self.journal.persist(&self.path, &self.record, false)
    }

    pub(crate) fn destination_still_matches(&self) -> io::Result<bool> {
        let published = self.record.stage == TransferStage::Published;
        let path = if published {
            self.record.destination()
        } else {
            self.record.staging_destination()
        };
        self.record.destination_path_matches(&path, published)
    }

    pub(crate) fn mark_source_removed(&mut self) -> io::Result<()> {
        if self.record.operation != JournalOperation::Move {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "copy journal cannot remove its source",
            ));
        }
        match fs::symlink_metadata(self.record.source()) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "source path still exists after removal",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.record.stage = TransferStage::SourceRemoved;
        self.journal.persist(&self.path, &self.record, false)
    }

    pub(crate) fn publish(&mut self) -> io::Result<()> {
        if self.record.operation != JournalOperation::Move
            || self.record.stage != TransferStage::SourceRemoved
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "move cannot publish before source removal",
            ));
        }
        self.publish_entry()
    }

    pub(crate) fn publish_copy(&mut self) -> io::Result<()> {
        if self.record.operation != JournalOperation::Copy
            || self.record.stage != TransferStage::DestinationComplete
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "copy is not ready for publication",
            ));
        }
        self.publish_entry()
    }

    fn publish_preserved(&mut self) -> io::Result<()> {
        if self.record.stage != TransferStage::DestinationComplete
            || self.record.resolution_intent != Some(ResolutionIntent::PreserveCopy)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery copy is not ready for publication",
            ));
        }
        self.publish_entry()
    }

    fn publish_entry(&mut self) -> io::Result<()> {
        if !self.destination_still_matches()? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "staged destination identity changed before publication",
            ));
        }
        rename_noreplace(
            &self.record.staging_destination(),
            &self.record.destination(),
        )?;
        sync_directory(
            self.record
                .destination()
                .parent()
                .ok_or_else(|| invalid_data("transfer destination has no parent"))?,
        )?;
        let published_identity = EntryIdentity::capture(&self.record.destination())?;
        if !self
            .record
            .destination_identity
            .as_ref()
            .is_some_and(|staged| staged.same_entry_after_rename(&published_identity))
        {
            return Err(invalid_data(
                "published destination identity changed unexpectedly",
            ));
        }
        if let Some(expected) = &self.record.destination_manifest {
            let published_manifest = TreeManifest::capture(&self.record.destination())?;
            if !expected.same_after_root_rename(&published_manifest) {
                return Err(invalid_data(
                    "published destination tree changed unexpectedly",
                ));
            }
        }
        self.record.destination_identity = Some(published_identity);
        self.record.stage = TransferStage::Published;
        self.journal.persist(&self.path, &self.record, false)
    }

    pub(crate) fn commit(self) -> io::Result<()> {
        if self.record.stage != TransferStage::Published {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "transfer journal cannot commit before destination publication",
            ));
        }
        self.journal.finish_record(&self.path, &self.lock)
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn capture_optional(path: &Path) -> io::Result<Option<EntryIdentity>> {
    match EntryIdentity::capture(path) {
        Ok(identity) => Ok(Some(identity)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn available_recovery_destination(destination: &Path) -> io::Result<PathBuf> {
    if capture_optional(destination)?.is_none() {
        return Ok(destination.to_path_buf());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| invalid_data("recovery destination has no parent"))?;
    let name = destination
        .file_name()
        .ok_or_else(|| invalid_data("recovery destination has no file name"))?;
    let mut base = name.as_bytes().to_vec();
    base.extend_from_slice(b" (Recovered)");
    for index in 1..=10_000 {
        let mut bytes = base.clone();
        if index > 1 {
            bytes.extend_from_slice(format!(" {index}").as_bytes());
        }
        let candidate = parent.join(OsString::from_vec(bytes));
        if capture_optional(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no recovery destination name is available",
    ))
}

fn review_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "file-operation recovery changed; review it again",
    )
}

fn lock_record_id(name: &str) -> Option<&str> {
    name.strip_suffix(".lock")
}

fn temp_record_id(name: &str) -> Option<&str> {
    let body = name.strip_prefix('.')?.strip_suffix(".tmp")?;
    let (record, nonce) = body.split_once('.')?;
    (Uuid::parse_str(record).is_ok() && Uuid::parse_str(nonce).is_ok()).then_some(record)
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

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rmac-files-journal-{label}-{}-{unique}",
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

    #[test]
    fn prepared_move_survives_reopening_without_exposing_paths() {
        let root = TestDirectory::new("prepare");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal_root = root.0.join("journal");
        let journal = Journal::open(journal_root.clone()).unwrap();

        let _ticket = journal.prepare_move(&source, &destination).unwrap();

        assert_eq!(
            Journal::open(journal_root)
                .unwrap()
                .pending_count()
                .unwrap(),
            1
        );
    }

    #[test]
    fn second_instance_skips_an_active_operation() {
        let root = TestDirectory::new("active-record");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal_root = root.0.join("journal");
        let first = Journal::open(journal_root.clone()).unwrap();
        let ticket = first.prepare_move(&source, &destination).unwrap();
        let second = Journal::open(journal_root).unwrap();

        let active = second.recover_unambiguous().unwrap();
        let active_reviews = second.review_pending().unwrap();

        assert_eq!(active.finalized, 0);
        assert_eq!(active.pending, 0);
        assert_eq!(active.active, 1);
        assert!(active_reviews.is_empty());

        drop(ticket);
        let abandoned = second.recover_unambiguous().unwrap();
        assert_eq!(abandoned.active, 0);
        assert_eq!(abandoned.pending, 1);
        assert_eq!(second.review_pending().unwrap().len(), 1);
    }

    #[test]
    fn active_journal_temporary_is_not_collected_by_another_instance() {
        let root = TestDirectory::new("active-temporary");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal_root = root.0.join("journal");
        let first = Journal::open(journal_root.clone()).unwrap();
        let ticket = first.prepare_move(&source, &destination).unwrap();
        let temporary = first
            .root
            .join(format!(".{}.{}.tmp", ticket.record.id, Uuid::new_v4()));
        let mut temporary_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .unwrap();
        temporary_file.write_all(b"active atomic update").unwrap();
        temporary_file.sync_all().unwrap();
        drop(temporary_file);

        let _second = Journal::open(journal_root.clone()).unwrap();

        assert!(temporary.exists());
        drop(ticket);
        let _third = Journal::open(journal_root).unwrap();
        assert!(!temporary.exists());
    }

    #[test]
    fn orphan_lock_is_removed_only_after_its_owner_exits() {
        let root = TestDirectory::new("orphan-lock");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal_root = root.0.join("journal");
        let first = Journal::open(journal_root.clone()).unwrap();
        let ticket = first.prepare_move(&source, &destination).unwrap();
        let lock_path = ticket.lock.path.clone();
        fs::remove_file(&ticket.path).unwrap();

        let _second = Journal::open(journal_root.clone()).unwrap();
        assert!(lock_path.exists());

        drop(ticket);
        let _third = Journal::open(journal_root).unwrap();
        assert!(!lock_path.exists());
    }

    #[test]
    fn review_resolution_refuses_a_record_locked_after_review() {
        let root = TestDirectory::new("review-lock-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        let id = ticket.record.id.clone();
        drop(ticket);
        let review = journal.review_pending().unwrap().remove(0);
        let competing_lock = journal.try_lock_record(&id).unwrap().unwrap();

        let error = journal.resolve_review(&review).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 1);

        drop(competing_lock);
        assert_eq!(
            journal.resolve_review(&review).unwrap(),
            ResolutionOutcome::KeptExistingItems
        );
    }

    #[test]
    fn symlinked_record_lock_fails_closed() {
        let root = TestDirectory::new("symlink-lock");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        let lock_path = ticket.lock.path.clone();
        let record_path = ticket.path.clone();
        drop(ticket);
        fs::remove_file(&lock_path).unwrap();
        std::os::unix::fs::symlink(&source, &lock_path).unwrap();

        let error = journal.recover_unambiguous().unwrap_err();

        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert!(!destination.exists());
        assert!(record_path.exists());
    }

    #[test]
    fn successful_move_lifecycle_removes_the_durable_record() {
        let root = TestDirectory::new("commit");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        let lock_path = ticket.lock.path.clone();
        let lock_metadata = fs::symlink_metadata(&lock_path).unwrap();
        assert!(lock_metadata.is_file());
        assert_eq!(lock_metadata.mode() & 0o777, 0o600);
        assert_eq!(lock_metadata.len(), 0);
        fs::write(ticket.staging_destination(), b"source").unwrap();

        ticket.mark_destination_complete().unwrap();
        assert!(ticket.destination_still_matches().unwrap());
        assert!(ticket.source_still_matches().unwrap());
        fs::remove_file(&source).unwrap();
        ticket.mark_source_removed().unwrap();
        ticket.publish().unwrap();
        ticket.commit().unwrap();

        assert_eq!(journal.pending_count().unwrap(), 0);
        assert_eq!(fs::read(&destination).unwrap(), b"source");
        assert!(!lock_path.exists());
    }

    #[test]
    fn successful_copy_lifecycle_preserves_source_and_removes_record() {
        let root = TestDirectory::new("copy-commit");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        fs::write(ticket.staging_destination(), b"source").unwrap();

        ticket.mark_destination_complete().unwrap();
        assert!(ticket.source_still_matches().unwrap());
        assert!(ticket.destination_still_matches().unwrap());
        ticket.publish_copy().unwrap();
        ticket.commit().unwrap();

        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn recovery_publishes_identity_proven_regular_copy() {
        let root = TestDirectory::new("copy-recover");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        fs::write(ticket.staging_destination(), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(
            report,
            RecoveryReport {
                finalized: 1,
                pending: 0,
                active: 0,
            }
        );
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn recovery_finishes_copy_published_before_stage_persisted() {
        let root = TestDirectory::new("copy-publish-interruption");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        rename_noreplace(&staging, &destination).unwrap();
        sync_directory(&root.0).unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn changed_copy_source_keeps_complete_stage_for_review() {
        let root = TestDirectory::new("copy-source-change");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::write(&source, b"changed source bytes").unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();
        let review = journal.review_pending().unwrap().remove(0);

        assert_eq!(report.finalized, 0);
        assert_eq!(report.pending, 1);
        assert_eq!(
            review.action,
            RecoveryAction::PreserveCopy {
                complete: false,
                suggested_name: "destination".to_string(),
            }
        );
        assert!(!destination.exists());
        assert_eq!(fs::read(staging).unwrap(), b"source");
        assert_eq!(fs::read(source).unwrap(), b"changed source bytes");
    }

    #[test]
    fn recovery_publishes_unchanged_manifest_proven_directory_copy() {
        let root = TestDirectory::new("copy-directory-recovery");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("item"), b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("item"), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(fs::read(destination.join("item")).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn nested_source_mutation_invalidates_directory_copy_manifest() {
        let root = TestDirectory::new("copy-directory-mutation");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("item"), b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("item"), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::write(source.join("item"), b"changed nested bytes").unwrap();

        assert!(!ticket.source_still_matches().unwrap());
        drop(ticket);
        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 0);
        assert_eq!(report.pending, 1);
        assert!(!destination.exists());
        assert_eq!(fs::read(staging.join("item")).unwrap(), b"source");
    }

    #[test]
    fn nested_staging_mutation_invalidates_destination_manifest() {
        let root = TestDirectory::new("copy-staging-mutation");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("item"), b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("item"), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::write(staging.join("item"), b"substituted stage").unwrap();

        assert!(!ticket.destination_still_matches().unwrap());
        drop(ticket);
        let report = journal.recover_unambiguous().unwrap();
        let review = journal.review_pending().unwrap().remove(0);

        assert_eq!(report.finalized, 0);
        assert_eq!(report.pending, 1);
        assert_eq!(
            review.action,
            RecoveryAction::PreserveCopy {
                complete: false,
                suggested_name: "destination".to_string(),
            }
        );
        assert!(!destination.exists());
        assert_eq!(
            fs::read(staging.join("item")).unwrap(),
            b"substituted stage"
        );
    }

    #[test]
    fn source_manifest_is_deterministic_and_never_follows_symlink_target() {
        let root = TestDirectory::new("manifest-symlink");
        let source = root.0.join("source");
        let outside = root.0.join("outside");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(source.join("z"), b"last").unwrap();
        fs::write(source.join("a"), b"first").unwrap();
        fs::write(outside.join("private"), b"private one").unwrap();
        std::os::unix::fs::symlink(&outside, source.join("link")).unwrap();

        let first = TreeManifest::capture(&source).unwrap();
        let second = TreeManifest::capture(&source).unwrap();
        fs::write(outside.join("private"), b"private target changed").unwrap();
        let after_target_change = TreeManifest::capture(&source).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            first, after_target_change,
            "manifest must bind the link itself, not private target content"
        );

        fs::remove_file(source.join("link")).unwrap();
        std::os::unix::fs::symlink("different-target", source.join("link")).unwrap();
        assert_ne!(first, TreeManifest::capture(&source).unwrap());
    }

    #[test]
    fn nested_source_change_invalidates_copy_recovery_review() {
        let root = TestDirectory::new("copy-review-manifest-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("item"), b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("item"), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::write(&destination, b"conflict").unwrap();
        drop(ticket);
        let review = journal.review_pending().unwrap().remove(0);
        assert_eq!(
            review.action,
            RecoveryAction::PreserveCopy {
                complete: true,
                suggested_name: "destination (Recovered)".to_string(),
            }
        );
        fs::write(source.join("item"), b"changed after review").unwrap();

        let error = journal.resolve_review(&review).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(destination).unwrap(), b"conflict");
        assert_eq!(fs::read(staging.join("item")).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn nested_stage_change_invalidates_copy_recovery_review() {
        let root = TestDirectory::new("copy-review-stage-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("item"), b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("item"), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::write(&destination, b"conflict").unwrap();
        drop(ticket);
        let review = journal.review_pending().unwrap().remove(0);
        assert!(matches!(
            review.action,
            RecoveryAction::PreserveCopy { complete: true, .. }
        ));
        fs::write(staging.join("item"), b"substituted after review").unwrap();

        let error = journal.resolve_review(&review).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(destination).unwrap(), b"conflict");
        assert_eq!(
            fs::read(staging.join("item")).unwrap(),
            b"substituted after review"
        );
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn copy_recovery_never_replaces_a_conflicting_destination() {
        let root = TestDirectory::new("copy-recovery-conflict");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_copy(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::write(&destination, b"conflict").unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 0);
        assert_eq!(report.pending, 1);
        assert_eq!(fs::read(destination).unwrap(), b"conflict");
        assert_eq!(fs::read(staging).unwrap(), b"source");
    }

    #[test]
    fn replaced_source_identity_is_detected_before_removal() {
        let root = TestDirectory::new("replacement");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"original").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        fs::write(ticket.staging_destination(), b"original").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::remove_file(&source).unwrap();
        fs::write(&source, b"replacement").unwrap();

        assert!(!ticket.source_still_matches().unwrap());
        assert_eq!(fs::read(&source).unwrap(), b"replacement");
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn journal_round_trips_non_utf8_paths() {
        let source = PathBuf::from("/").join(OsString::from_vec(vec![b's', b'o', b'u', 0xff]));
        let destination =
            PathBuf::from("/").join(OsString::from_vec(vec![b'd', b'e', b's', b't', 0xfe]));
        let id = Uuid::new_v4().to_string();
        let record = TransferRecord {
            version: JOURNAL_VERSION,
            id: id.clone(),
            operation: JournalOperation::Move,
            stage: TransferStage::Prepared,
            source_path_bytes: source.as_os_str().as_bytes().to_vec(),
            destination_path_bytes: destination.as_os_str().as_bytes().to_vec(),
            staging_path_bytes: PathBuf::from("/")
                .join(format!(".rmac-transfer-{id}"))
                .as_os_str()
                .as_bytes()
                .to_vec(),
            source_identity: EntryIdentity {
                device: 1,
                inode: 2,
                mode: 0o100600,
                size: 3,
                modified_seconds: 4,
                modified_nanoseconds: 5,
                changed_seconds: 6,
                changed_nanoseconds: 7,
            },
            source_manifest: None,
            destination_identity: None,
            destination_manifest: None,
            resolution_intent: None,
        };

        let bytes = serde_json::to_vec(&record).unwrap();
        let decoded: TransferRecord = serde_json::from_slice(&bytes).unwrap();

        decoded.validate(&id).unwrap();
        assert_eq!(decoded.operation, JournalOperation::Move);
        assert_eq!(decoded.source(), source);
        assert_eq!(decoded.destination(), destination);

        let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let legacy_object = legacy.as_object_mut().unwrap();
        legacy_object.remove("operation");
        legacy_object.remove("source_manifest");
        let legacy: TransferRecord = serde_json::from_value(legacy).unwrap();
        legacy.validate(&id).unwrap();
        assert_eq!(legacy.operation, JournalOperation::Move);
    }

    #[test]
    fn malformed_record_fails_closed() {
        let root = TestDirectory::new("malformed");
        let journal = Journal::open(root.0.join("journal")).unwrap();
        fs::write(journal.root.join(format!("{}.json", Uuid::new_v4())), b"{}").unwrap();

        let error = journal.pending_count().unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn recovery_publishes_a_complete_copy_after_source_removal() {
        let root = TestDirectory::new("recover-source-removed");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        fs::write(ticket.staging_destination(), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::remove_file(&source).unwrap();
        ticket.mark_source_removed().unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(
            report,
            RecoveryReport {
                finalized: 1,
                pending: 0,
                active: 0,
            }
        );
        assert_eq!(fs::read(destination).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn recovery_infers_source_removal_after_an_interrupted_stage_write() {
        let root = TestDirectory::new("recover-stale-stage");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        fs::write(ticket.staging_destination(), b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::remove_file(&source).unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(fs::read(destination).unwrap(), b"source");
    }

    #[test]
    fn recovery_never_overwrites_a_conflicting_final_path() {
        let root = TestDirectory::new("recover-conflict");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::remove_file(&source).unwrap();
        ticket.mark_source_removed().unwrap();
        fs::write(&destination, b"conflict").unwrap();
        drop(ticket);

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 0);
        assert_eq!(report.pending, 1);
        assert_eq!(fs::read(destination).unwrap(), b"conflict");
        assert_eq!(fs::read(staging).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn reviewed_conflict_preserves_complete_copy_under_a_new_name() {
        let root = TestDirectory::new("review-complete");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"source").unwrap();
        ticket.mark_destination_complete().unwrap();
        fs::remove_file(&source).unwrap();
        ticket.mark_source_removed().unwrap();
        fs::write(&destination, b"conflict").unwrap();
        drop(ticket);

        let reviews = journal.review_pending().unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(
            reviews[0].action,
            RecoveryAction::PreserveCopy {
                complete: true,
                suggested_name: "destination (Recovered)".to_string(),
            }
        );
        assert!(!format!("{:?}", reviews[0]).contains("destination"));

        let outcome = journal.resolve_review(&reviews[0]).unwrap();

        assert_eq!(
            outcome,
            ResolutionOutcome::PreservedCopy {
                complete: true,
                name: "destination (Recovered)".to_string(),
            }
        );
        assert_eq!(fs::read(&destination).unwrap(), b"conflict");
        assert_eq!(
            fs::read(root.0.join("destination (Recovered)")).unwrap(),
            b"source"
        );
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn reviewed_partial_copy_is_preserved_without_removing_source() {
        let root = TestDirectory::new("review-partial");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        fs::write(ticket.staging_destination(), b"partial").unwrap();
        drop(ticket);

        let review = journal.review_pending().unwrap().remove(0);
        assert_eq!(
            review.action,
            RecoveryAction::PreserveCopy {
                complete: false,
                suggested_name: "destination".to_string(),
            }
        );

        let outcome = journal.resolve_review(&review).unwrap();

        assert_eq!(
            outcome,
            ResolutionOutcome::PreservedCopy {
                complete: false,
                name: "destination".to_string(),
            }
        );
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&destination).unwrap(), b"partial");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn reviewed_record_without_staged_data_keeps_existing_items() {
        let root = TestDirectory::new("review-no-stage");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        drop(ticket);

        let review = journal.review_pending().unwrap().remove(0);
        assert_eq!(review.action, RecoveryAction::KeepExistingItems);

        let outcome = journal.resolve_review(&review).unwrap();

        assert_eq!(outcome, ResolutionOutcome::KeptExistingItems);
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert!(!destination.exists());
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn substituted_recovery_copy_invalidates_the_review() {
        let root = TestDirectory::new("review-substitution");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"partial").unwrap();
        drop(ticket);
        let review = journal.review_pending().unwrap().remove(0);
        fs::remove_file(&staging).unwrap();
        fs::write(&staging, b"substituted recovery bytes").unwrap();

        let error = journal.resolve_review(&review).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(staging).unwrap(), b"substituted recovery bytes");
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn racing_recovery_name_never_gets_replaced() {
        let root = TestDirectory::new("review-name-race");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"existing destination").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"recovery").unwrap();
        drop(ticket);
        let review = journal.review_pending().unwrap().remove(0);
        let candidate = review.preserve_destination.clone().unwrap();
        fs::write(&candidate, b"racing recovery name").unwrap();

        let error = journal.resolve_review(&review).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&destination).unwrap(), b"existing destination");
        assert_eq!(fs::read(&candidate).unwrap(), b"racing recovery name");
        assert_eq!(fs::read(&staging).unwrap(), b"recovery");
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn persisted_preserve_intent_recovers_after_interruption() {
        let root = TestDirectory::new("review-intent-recovery");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"conflict").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let ticket = journal.prepare_move(&source, &destination).unwrap();
        let staging = ticket.staging_destination();
        fs::write(&staging, b"recovery").unwrap();
        drop(ticket);
        let review = journal.review_pending().unwrap().remove(0);
        let candidate = review.preserve_destination.clone().unwrap();
        let mut record = review.record.clone();
        record.destination_path_bytes = candidate.as_os_str().as_bytes().to_vec();
        record.destination_identity = review.staging_snapshot.clone();
        record.destination_manifest =
            Some(TreeManifest::capture(&record.staging_destination()).unwrap());
        record.stage = TransferStage::DestinationComplete;
        record.resolution_intent = Some(ResolutionIntent::PreserveCopy);
        journal
            .persist(&journal.record_path(&record.id), &record, false)
            .unwrap();

        let report = journal.recover_unambiguous().unwrap();

        assert_eq!(report.finalized, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"conflict");
        assert_eq!(fs::read(candidate).unwrap(), b"recovery");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }
}
