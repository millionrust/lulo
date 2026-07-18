use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rmac_notes_store::{
    MutationError, NoteChanges, NoteId, MAX_BODY_BYTES, MAX_TAGS_PER_NOTE, MAX_TAG_BYTES,
    MAX_TITLE_BYTES,
};
use rmac_storage::{Backend, FileSystem};

const MAGIC: &[u8; 8] = b"RMNDRFT\0";
const VERSION: u16 = 1;
const DRAFT_DIRECTORY: &str = "drafts";
const DRAFT_PREFIX: &str = "note-";
const DRAFT_SUFFIX: &str = ".draft";
const FIXED_RECORD_BYTES: usize = MAGIC.len() + 2 + (8 * 5) + 4 + 4 + 2;

pub const MAX_DRAFT_RECORD_BYTES: usize =
    FIXED_RECORD_BYTES + MAX_TITLE_BYTES + MAX_BODY_BYTES + MAX_TAGS_PER_NOTE * (2 + MAX_TAG_BYTES);
pub const MAX_DISCOVERED_DRAFTS: usize = 256;
pub const MAX_DRAFT_DISCOVERY_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_SCANNED_DRAFT_ENTRIES: usize = 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct DraftRecord {
    pub note_id: NoteId,
    pub base_note_revision: u64,
    pub edit_generation: u64,
    pub updated_unix_ms: u64,
    pub changes: NoteChanges,
}

impl DraftRecord {
    pub fn new(
        note_id: NoteId,
        base_note_revision: u64,
        edit_generation: u64,
        updated_unix_ms: u64,
        changes: NoteChanges,
    ) -> Result<Self, DraftCodecError> {
        if base_note_revision == 0 || edit_generation == 0 {
            return Err(DraftCodecError::InvalidField);
        }
        changes
            .validate_content()
            .map_err(DraftCodecError::InvalidContent)?;
        Ok(Self {
            note_id,
            base_note_revision,
            edit_generation,
            updated_unix_ms,
            changes,
        })
    }
}

impl fmt::Debug for DraftRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DraftRecord")
            .field("note_id", &self.note_id)
            .field("base_note_revision", &self.base_note_revision)
            .field("edit_generation", &self.edit_generation)
            .field("updated_unix_ms", &self.updated_unix_ms)
            .field("changes", &"[private]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftCodecError {
    InvalidHeader,
    UnsupportedVersion,
    InvalidField,
    InvalidUtf8,
    Truncated,
    TrailingData,
    InvalidContent(MutationError),
}

impl fmt::Display for DraftCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHeader => "the Notes draft header is invalid",
            Self::UnsupportedVersion => "the Notes draft version is not supported",
            Self::InvalidField => "the Notes draft contains an invalid field",
            Self::InvalidUtf8 => "the Notes draft contains invalid text encoding",
            Self::Truncated => "the Notes draft ended unexpectedly",
            Self::TrailingData => "the Notes draft contains undeclared trailing data",
            Self::InvalidContent(_) => "the Notes draft content is invalid",
        })
    }
}

impl std::error::Error for DraftCodecError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftOperation {
    PrepareDirectory,
    Encode,
    Read,
    Write,
    Verify,
    Remove,
    VerifyRemoval,
    Discover,
    Quarantine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftErrorKind {
    Io(io::ErrorKind),
    Codec(DraftCodecError),
    ReadbackMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftError {
    pub operation: DraftOperation,
    pub kind: DraftErrorKind,
}

impl DraftError {
    fn io(operation: DraftOperation, error: io::Error) -> Self {
        Self {
            operation,
            kind: DraftErrorKind::Io(error.kind()),
        }
    }

    fn codec(operation: DraftOperation, error: DraftCodecError) -> Self {
        Self {
            operation,
            kind: DraftErrorKind::Codec(error),
        }
    }
}

impl fmt::Display for DraftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.operation {
            DraftOperation::PrepareDirectory => "Notes could not prepare private draft storage",
            DraftOperation::Encode => "Notes could not validate the recovery draft",
            DraftOperation::Read | DraftOperation::Discover => {
                "Notes could not inspect private recovery drafts"
            }
            DraftOperation::Write | DraftOperation::Verify => {
                "Notes could not safely retain the newest recovery draft"
            }
            DraftOperation::Remove | DraftOperation::VerifyRemoval => {
                "Notes could not remove a committed recovery draft"
            }
            DraftOperation::Quarantine => "Notes could not isolate a malformed recovery draft",
        })
    }
}

impl std::error::Error for DraftError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftDiscovery {
    pub drafts: Vec<DraftRecord>,
    pub malformed: usize,
    pub quarantined: usize,
    pub excessive: bool,
    pub unavailable: bool,
}

pub struct DraftStore<B = FileSystem> {
    directory: PathBuf,
    backend: B,
}

impl DraftStore<FileSystem> {
    pub fn for_library(library_root: &Path) -> Self {
        Self::new(library_root, FileSystem)
    }

    pub fn discover(&self) -> DraftDiscovery {
        discover_directory(&self.directory)
    }
}

impl<B: Backend> DraftStore<B> {
    pub fn new(library_root: &Path, backend: B) -> Self {
        Self {
            directory: library_root.join(DRAFT_DIRECTORY),
            backend,
        }
    }

    pub fn save(&self, record: &DraftRecord) -> Result<(), DraftError> {
        let encoded = encode_draft(record)
            .map_err(|error| DraftError::codec(DraftOperation::Encode, error))?;
        self.backend
            .create_dir_all_private(&self.directory)
            .map_err(|error| DraftError::io(DraftOperation::PrepareDirectory, error))?;
        let path = self.path(record.note_id);
        self.backend
            .write_atomic_private(&path, &encoded)
            .map_err(|error| DraftError::io(DraftOperation::Write, error))?;
        let readback = self
            .backend
            .read_bounded_no_follow(&path, MAX_DRAFT_RECORD_BYTES)
            .map_err(|error| DraftError::io(DraftOperation::Verify, error))?;
        let decoded = decode_draft(&readback)
            .map_err(|error| DraftError::codec(DraftOperation::Verify, error))?;
        if readback != encoded || decoded != *record {
            return Err(DraftError {
                operation: DraftOperation::Verify,
                kind: DraftErrorKind::ReadbackMismatch,
            });
        }
        Ok(())
    }

    pub fn load(&self, note_id: NoteId) -> Result<Option<DraftRecord>, DraftError> {
        let path = self.path(note_id);
        let bytes = match self
            .backend
            .read_bounded_no_follow(&path, MAX_DRAFT_RECORD_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(DraftError::io(DraftOperation::Read, error)),
        };
        let draft =
            decode_draft(&bytes).map_err(|error| DraftError::codec(DraftOperation::Read, error))?;
        if draft.note_id != note_id {
            return Err(DraftError::codec(
                DraftOperation::Read,
                DraftCodecError::InvalidField,
            ));
        }
        Ok(Some(draft))
    }

    pub fn remove(&self, note_id: NoteId) -> Result<(), DraftError> {
        let path = self.path(note_id);
        match self.backend.remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(DraftError::io(DraftOperation::Remove, error)),
        }
        match self
            .backend
            .read_bounded_no_follow(&path, MAX_DRAFT_RECORD_BYTES)
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(DraftError::io(DraftOperation::VerifyRemoval, error)),
            Ok(_) => Err(DraftError {
                operation: DraftOperation::VerifyRemoval,
                kind: DraftErrorKind::ReadbackMismatch,
            }),
        }
    }

    fn path(&self, note_id: NoteId) -> PathBuf {
        self.directory.join(draft_name(note_id))
    }
}

impl<B> fmt::Debug for DraftStore<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("DraftStore").finish_non_exhaustive()
    }
}

pub fn encode_draft(record: &DraftRecord) -> Result<Vec<u8>, DraftCodecError> {
    let validated = DraftRecord::new(
        record.note_id,
        record.base_note_revision,
        record.edit_generation,
        record.updated_unix_ms,
        record.changes.clone(),
    )?;
    let title_len =
        u32::try_from(validated.changes.title.len()).map_err(|_| DraftCodecError::InvalidField)?;
    let body_len =
        u32::try_from(validated.changes.body.len()).map_err(|_| DraftCodecError::InvalidField)?;
    let tag_count =
        u16::try_from(validated.changes.tags.len()).map_err(|_| DraftCodecError::InvalidField)?;
    let mut output = Vec::with_capacity(
        FIXED_RECORD_BYTES
            .saturating_add(validated.changes.title.len())
            .saturating_add(validated.changes.body.len())
            .saturating_add(
                validated
                    .changes
                    .tags
                    .iter()
                    .map(|tag| 2_usize.saturating_add(tag.len()))
                    .sum::<usize>(),
            ),
    );
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&VERSION.to_le_bytes());
    output.extend_from_slice(&validated.note_id.get().to_le_bytes());
    output.extend_from_slice(&validated.base_note_revision.to_le_bytes());
    output.extend_from_slice(&validated.edit_generation.to_le_bytes());
    output.extend_from_slice(&validated.updated_unix_ms.to_le_bytes());
    output.extend_from_slice(&validated.changes.modified_unix_ms.to_le_bytes());
    output.extend_from_slice(&title_len.to_le_bytes());
    output.extend_from_slice(&body_len.to_le_bytes());
    output.extend_from_slice(&tag_count.to_le_bytes());
    output.extend_from_slice(validated.changes.title.as_bytes());
    output.extend_from_slice(validated.changes.body.as_bytes());
    for tag in &validated.changes.tags {
        let length = u16::try_from(tag.len()).map_err(|_| DraftCodecError::InvalidField)?;
        output.extend_from_slice(&length.to_le_bytes());
        output.extend_from_slice(tag.as_bytes());
    }
    if output.len() > MAX_DRAFT_RECORD_BYTES {
        return Err(DraftCodecError::InvalidField);
    }
    Ok(output)
}

pub fn decode_draft(bytes: &[u8]) -> Result<DraftRecord, DraftCodecError> {
    if bytes.len() > MAX_DRAFT_RECORD_BYTES || !bytes.starts_with(MAGIC) {
        return Err(DraftCodecError::InvalidHeader);
    }
    let mut cursor = MAGIC.len();
    let version = take_u16(bytes, &mut cursor)?;
    if version != VERSION {
        return Err(DraftCodecError::UnsupportedVersion);
    }
    let note_id =
        NoteId::new(take_u64(bytes, &mut cursor)?).ok_or(DraftCodecError::InvalidField)?;
    let base_note_revision = take_u64(bytes, &mut cursor)?;
    let edit_generation = take_u64(bytes, &mut cursor)?;
    let updated_unix_ms = take_u64(bytes, &mut cursor)?;
    let modified_unix_ms = take_u64(bytes, &mut cursor)?;
    let title_len = usize::try_from(take_u32(bytes, &mut cursor)?)
        .map_err(|_| DraftCodecError::InvalidField)?;
    let body_len = usize::try_from(take_u32(bytes, &mut cursor)?)
        .map_err(|_| DraftCodecError::InvalidField)?;
    let tag_count = usize::from(take_u16(bytes, &mut cursor)?);
    if title_len > MAX_TITLE_BYTES || body_len > MAX_BODY_BYTES || tag_count > MAX_TAGS_PER_NOTE {
        return Err(DraftCodecError::InvalidField);
    }
    let title = take_utf8(bytes, &mut cursor, title_len)?;
    let body = take_utf8(bytes, &mut cursor, body_len)?;
    let mut tags = Vec::with_capacity(tag_count);
    for _ in 0..tag_count {
        let length = usize::from(take_u16(bytes, &mut cursor)?);
        if length > MAX_TAG_BYTES {
            return Err(DraftCodecError::InvalidField);
        }
        tags.push(take_utf8(bytes, &mut cursor, length)?);
    }
    if cursor != bytes.len() {
        return Err(DraftCodecError::TrailingData);
    }
    DraftRecord::new(
        note_id,
        base_note_revision,
        edit_generation,
        updated_unix_ms,
        NoteChanges {
            modified_unix_ms,
            title,
            body,
            tags,
        },
    )
}

fn discover_directory(directory: &Path) -> DraftDiscovery {
    let mut discovery = DraftDiscovery::default();
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return discovery,
        Err(_) => {
            discovery.unavailable = true;
            return discovery;
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            discovery.unavailable = true;
            return discovery;
        }
        Ok(_) => {}
    }
    if rmac_storage::create_dir_all_private(directory).is_err() {
        discovery.unavailable = true;
        return discovery;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => {
            discovery.unavailable = true;
            return discovery;
        }
    };
    let mut paths = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCANNED_DRAFT_ENTRIES {
            discovery.excessive = true;
            break;
        }
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(_) => discovery.unavailable = true,
        }
    }
    paths.sort();
    let mut retained_bytes = 0_u64;
    for path in paths {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(DRAFT_PREFIX) || !name.ends_with(DRAFT_SUFFIX) {
            continue;
        }
        if discovery.drafts.len() >= MAX_DISCOVERED_DRAFTS {
            discovery.excessive = true;
            continue;
        }
        let expected_note_id = parse_draft_name(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                discovery.unavailable = true;
                continue;
            }
        };
        if expected_note_id.is_none()
            || !metadata.file_type().is_file()
            || metadata.len() > MAX_DRAFT_RECORD_BYTES as u64
        {
            mark_malformed(&mut discovery, &path);
            continue;
        }
        let Some(next_bytes) = retained_bytes.checked_add(metadata.len()) else {
            discovery.excessive = true;
            continue;
        };
        if next_bytes > MAX_DRAFT_DISCOVERY_BYTES {
            discovery.excessive = true;
            continue;
        }
        let bytes = match rmac_storage::read_bounded_no_follow(&path, MAX_DRAFT_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound
                ) =>
            {
                discovery.unavailable = true;
                continue;
            }
            Err(_) => {
                mark_malformed(&mut discovery, &path);
                continue;
            }
        };
        match decode_draft(&bytes) {
            Ok(record) if Some(record.note_id) == expected_note_id => {
                retained_bytes = next_bytes;
                discovery.drafts.push(record);
            }
            Ok(_) | Err(_) => mark_malformed(&mut discovery, &path),
        }
    }
    discovery.drafts.sort_by(|left, right| {
        right
            .updated_unix_ms
            .cmp(&left.updated_unix_ms)
            .then_with(|| left.note_id.cmp(&right.note_id))
    });
    discovery
}

fn mark_malformed(discovery: &mut DraftDiscovery, path: &Path) {
    discovery.malformed = discovery.malformed.saturating_add(1);
    if quarantine(path) {
        discovery.quarantined = discovery.quarantined.saturating_add(1);
    } else {
        discovery.unavailable = true;
    }
}

fn quarantine(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    for suffix in 0..=MAX_SCANNED_DRAFT_ENTRIES {
        let quarantine_name = if suffix == 0 {
            format!("{name}.invalid")
        } else {
            format!("{name}.invalid-{suffix}")
        };
        let destination = path.with_file_name(quarantine_name);
        if destination.exists() {
            continue;
        }
        return fs::rename(path, destination).is_ok();
    }
    false
}

fn draft_name(note_id: NoteId) -> String {
    format!("{DRAFT_PREFIX}{:016x}{DRAFT_SUFFIX}", note_id.get())
}

fn parse_draft_name(name: &str) -> Option<NoteId> {
    let encoded = name
        .strip_prefix(DRAFT_PREFIX)?
        .strip_suffix(DRAFT_SUFFIX)?;
    if encoded.len() != 16 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let note_id = NoteId::new(u64::from_str_radix(encoded, 16).ok()?)?;
    (draft_name(note_id) == name).then_some(note_id)
}

fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, DraftCodecError> {
    Ok(u16::from_le_bytes(take_array(bytes, cursor)?))
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, DraftCodecError> {
    Ok(u32::from_le_bytes(take_array(bytes, cursor)?))
}

fn take_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, DraftCodecError> {
    Ok(u64::from_le_bytes(take_array(bytes, cursor)?))
}

fn take_array<const SIZE: usize>(
    bytes: &[u8],
    cursor: &mut usize,
) -> Result<[u8; SIZE], DraftCodecError> {
    let end = cursor.checked_add(SIZE).ok_or(DraftCodecError::Truncated)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(DraftCodecError::Truncated)?
        .try_into()
        .map_err(|_| DraftCodecError::Truncated)?;
    *cursor = end;
    Ok(value)
}

fn take_utf8(bytes: &[u8], cursor: &mut usize, length: usize) -> Result<String, DraftCodecError> {
    let end = cursor
        .checked_add(length)
        .ok_or(DraftCodecError::Truncated)?;
    let value = std::str::from_utf8(bytes.get(*cursor..end).ok_or(DraftCodecError::Truncated)?)
        .map_err(|_| DraftCodecError::InvalidUtf8)?
        .to_string();
    *cursor = end;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-notes-drafts-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn draft(note_id: u64, generation: u64, updated: u64, body: &str) -> DraftRecord {
        DraftRecord::new(
            NoteId::new(note_id).unwrap(),
            3,
            generation,
            updated,
            NoteChanges {
                modified_unix_ms: updated,
                title: "Private title".into(),
                body: body.into(),
                tags: vec!["private-tag".into()],
            },
        )
        .unwrap()
    }

    #[test]
    fn versioned_record_round_trips_and_rejects_malformed_data() {
        let record = draft(7, 9, 42, "Private body 🦀");
        let encoded = encode_draft(&record).unwrap();
        assert_eq!(decode_draft(&encoded).unwrap(), record);

        assert_eq!(
            decode_draft(b"not a draft"),
            Err(DraftCodecError::InvalidHeader)
        );
        let mut future = encoded.clone();
        future[MAGIC.len()..MAGIC.len() + 2].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            decode_draft(&future),
            Err(DraftCodecError::UnsupportedVersion)
        );
        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(decode_draft(&trailing), Err(DraftCodecError::TrailingData));

        let debug = format!("{record:?}");
        assert!(!debug.contains("Private title"));
        assert!(!debug.contains("Private body"));
        assert!(!debug.contains("private-tag"));
        assert!(debug.contains("[private]"));
        assert_eq!(
            parse_draft_name("note-0000000000000007.draft"),
            NoteId::new(7)
        );
        assert_eq!(parse_draft_name("note-000000000000000A.draft"), None);
    }

    #[cfg(unix)]
    #[test]
    fn save_load_and_remove_are_private_and_readback_verified() {
        use std::os::unix::fs::PermissionsExt as _;

        let library = root("round-trip");
        fs::create_dir(&library).unwrap();
        let store = DraftStore::for_library(&library);
        let record = draft(7, 9, 42, "Private body");

        store.save(&record).unwrap();

        assert_eq!(store.load(record.note_id).unwrap(), Some(record.clone()));
        let directory = library.join(DRAFT_DIRECTORY);
        let path = directory.join(draft_name(record.note_id));
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        store.remove(record.note_id).unwrap();
        assert_eq!(store.load(record.note_id).unwrap(), None);
        fs::remove_dir_all(library).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discovery_is_deterministic_and_quarantines_bad_or_linked_records() {
        use std::os::unix::fs::symlink;

        let library = root("discover");
        fs::create_dir(&library).unwrap();
        let store = DraftStore::for_library(&library);
        store.save(&draft(2, 1, 20, "newer")).unwrap();
        store.save(&draft(1, 1, 10, "older")).unwrap();
        let directory = library.join(DRAFT_DIRECTORY);
        fs::write(directory.join("note-bad.draft"), b"bad").unwrap();
        let linked = directory.join("note-0000000000000003.draft");
        symlink(directory.join(draft_name(NoteId::new(1).unwrap())), &linked).unwrap();

        let discovery = store.discover();

        assert_eq!(
            discovery
                .drafts
                .iter()
                .map(|draft| draft.note_id)
                .collect::<Vec<_>>(),
            vec![NoteId::new(2).unwrap(), NoteId::new(1).unwrap()]
        );
        assert_eq!(discovery.malformed, 2);
        assert_eq!(discovery.quarantined, 2);
        assert!(!discovery.unavailable);
        assert_eq!(
            store
                .load(NoteId::new(1).unwrap())
                .unwrap()
                .unwrap()
                .changes
                .body,
            "older"
        );
        fs::remove_dir_all(library).unwrap();
    }

    #[test]
    fn discovery_caps_retained_records_without_deleting_excess_drafts() {
        let library = root("excessive");
        fs::create_dir(&library).unwrap();
        let store = DraftStore::for_library(&library);
        for id in 1..=(MAX_DISCOVERED_DRAFTS as u64 + 1) {
            store.save(&draft(id, 1, id, "body")).unwrap();
        }

        let discovery = store.discover();

        assert_eq!(discovery.drafts.len(), MAX_DISCOVERED_DRAFTS);
        assert!(discovery.excessive);
        assert_eq!(
            fs::read_dir(library.join(DRAFT_DIRECTORY)).unwrap().count(),
            MAX_DISCOVERED_DRAFTS + 1
        );
        fs::remove_dir_all(library).unwrap();
    }

    struct MemoryBackend {
        files: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
        corrupt_readback: bool,
    }

    impl Backend for MemoryBackend {
        fn read_bounded_no_follow(&self, path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
            let mut bytes = self
                .files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            if self.corrupt_readback && !bytes.is_empty() {
                bytes[0] ^= 0xff;
            }
            if bytes.len() > maximum {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            Ok(bytes)
        }

        fn write_atomic_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn create_dir_all_private(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.files
                .lock()
                .unwrap()
                .remove(path)
                .map(drop)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    #[test]
    fn corrupt_readback_never_reports_a_saved_draft() {
        let store = DraftStore::new(
            Path::new("/library"),
            MemoryBackend {
                files: Mutex::new(BTreeMap::new()),
                corrupt_readback: true,
            },
        );

        let error = store.save(&draft(1, 1, 1, "candidate")).unwrap_err();

        assert_eq!(error.operation, DraftOperation::Verify);
        assert!(matches!(
            error.kind,
            DraftErrorKind::Codec(_) | DraftErrorKind::ReadbackMismatch
        ));
    }
}
