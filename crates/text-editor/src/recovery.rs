use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::document::{LineEnding, TextEncoding, TextFormat, MAX_DOCUMENT_BYTES};
use crate::storage::{self, Failure, Storage};

const MAGIC: &[u8; 16] = b"RMAC-RECOVERY-v1";
const MAX_LABEL_BYTES: usize = 255;
const MAX_SOURCE_PATH_BYTES: usize = 4096;
const MAX_RECORD_BYTES: usize = MAX_DOCUMENT_BYTES + 8192;
const MAX_DISCOVERED_RECORDS: usize = 64;
const MAX_DISCOVERY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SCANNED_ENTRIES: usize = 256;
const RECORD_PREFIX: &str = "draft-";
const RECORD_SUFFIX: &str = ".rmac-recovery";
const ACTIVE_OWNER_MARKER: &str = ".active-";
static RECORD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryRecord {
    pub(crate) created_unix_ms: u64,
    pub(crate) document_label: String,
    pub(crate) source_path: Option<String>,
    pub(crate) format: TextFormat,
    pub(crate) content: String,
}

impl RecoveryRecord {
    pub(crate) fn for_document(path: Option<&Path>, format: TextFormat, content: String) -> Self {
        let document_label = path
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Untitled".to_string());
        Self {
            created_unix_ms: now_millis(),
            document_label: bounded_display_text(&document_label, MAX_LABEL_BYTES),
            source_path: path
                .and_then(Path::to_str)
                .filter(|path| path.len() <= MAX_SOURCE_PATH_BYTES)
                .map(ToOwned::to_owned),
            format,
            content,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) path: PathBuf,
    pub(crate) record: RecoveryRecord,
}

#[derive(Default)]
pub(crate) struct Discovery {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) malformed: usize,
    pub(crate) excessive: bool,
    pub(crate) unavailable: bool,
}

pub(crate) fn fresh_record_path(directory: &Path) -> PathBuf {
    let sequence = RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    directory.join(format!(
        "{RECORD_PREFIX}{nanos}-{sequence}{ACTIVE_OWNER_MARKER}{}{RECORD_SUFFIX}",
        std::process::id(),
    ))
}

/// Atomically move an orphan record to a live process/window-specific identity.
/// Discovery ignores records whose filename names a live owner. If another
/// process claimed this path first, the rename fails and the caller can try the
/// next candidate without ever sharing a writable recovery path.
pub(crate) fn claim(directory: &Path, candidate: Candidate) -> Result<Candidate, Candidate> {
    let claimed_path = fresh_record_path(directory);
    match std::fs::rename(&candidate.path, &claimed_path) {
        Ok(()) => Ok(Candidate {
            path: claimed_path,
            record: candidate.record,
        }),
        Err(_) => Err(candidate),
    }
}

pub(crate) fn save(
    storage: &impl Storage,
    path: &Path,
    record: &RecoveryRecord,
) -> Result<(), Failure> {
    let encoded = encode(record)
        .map_err(|detail| Failure::message(storage::Operation::SaveRecovery, path, detail))?;
    storage::save_recovery(storage, storage::Operation::SaveRecovery, path, encoded)
}

pub(crate) fn discover(directory: &Path) -> Discovery {
    let mut discovery = Discovery::default();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return discovery,
        Err(_) => {
            discovery.unavailable = true;
            return discovery;
        }
    };
    let mut matching = 0_usize;
    let mut retained_bytes = 0_u64;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCANNED_ENTRIES {
            discovery.excessive = true;
            break;
        }
        let Ok(entry) = entry else {
            discovery.malformed += 1;
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(RECORD_PREFIX) || !name.ends_with(RECORD_SUFFIX) {
            continue;
        }
        if active_owner(name).is_some_and(process_is_alive) {
            continue;
        }
        matching += 1;
        if matching > MAX_DISCOVERED_RECORDS {
            discovery.excessive = true;
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            discovery.malformed += 1;
            continue;
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_RECORD_BYTES as u64 {
            discovery.malformed += 1;
            quarantine(&path);
            continue;
        }
        let Some(next_retained_bytes) = retained_bytes.checked_add(metadata.len()) else {
            discovery.excessive = true;
            continue;
        };
        if next_retained_bytes > MAX_DISCOVERY_BYTES {
            discovery.excessive = true;
            continue;
        }
        let decoded = rmac_storage::FileSystem
            .read_bounded(&path, MAX_RECORD_BYTES)
            .map_err(|_| ())
            .and_then(|bytes| decode(&bytes).map_err(|_| ()));
        match decoded {
            Ok(record) => {
                retained_bytes = next_retained_bytes;
                discovery.candidates.push(Candidate { path, record });
            }
            Err(()) => {
                discovery.malformed += 1;
                quarantine(&path);
            }
        }
    }
    if matching > MAX_DISCOVERED_RECORDS {
        discovery.excessive = true;
    }
    discovery.candidates.sort_by(|left, right| {
        right
            .record
            .created_unix_ms
            .cmp(&left.record.created_unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    discovery
}

fn active_owner(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(RECORD_SUFFIX)?;
    let (_, owner) = stem.rsplit_once(ACTIVE_OWNER_MARKER)?;
    owner.parse().ok()
}

#[cfg(unix)]
fn process_is_alive(process_id: u32) -> bool {
    let Ok(process_id) = i32::try_from(process_id) else {
        return false;
    };
    if process_id <= 0 {
        return false;
    }
    // SAFETY: signal 0 never delivers a signal; it only asks the kernel to
    // validate that the process exists and that the caller may signal it.
    let exists = unsafe { libc::kill(process_id, 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    exists && process_runs_this_program(process_id)
}

/// Process IDs start again from 1 after every boot, so the Text Editor that
/// owned a draft before a restart can share its number with an unrelated
/// process now. Only a process running this same program still owns it.
#[cfg(target_os = "linux")]
fn process_runs_this_program(process_id: i32) -> bool {
    let own = std::fs::read("/proc/self/comm").ok();
    let owner = std::fs::read(format!("/proc/{process_id}/comm")).ok();
    owner_runs_same_program(own.as_deref(), owner.as_deref())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_runs_this_program(_process_id: i32) -> bool {
    true
}

/// Whether a live owner process named `owner` is this program, named `own`.
/// When our own name cannot be read the owner is assumed live, so a record is
/// never taken from under a running editor; an owner whose name cannot be
/// read has already exited.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn owner_runs_same_program(own: Option<&[u8]>, owner: Option<&[u8]>) -> bool {
    match (own, owner) {
        (Some(own), Some(owner)) => own == owner,
        (None, Some(_)) => true,
        (_, None) => false,
    }
}

/// Orders one window's recovery writes. The debounced autosave writes on a
/// background thread while a session-end flush writes on the main thread;
/// each carries the document generation its text was read at, and a write
/// older than the newest one already on disk is skipped, so a slow autosave
/// can never replace a newer flushed draft, nor remove it afterwards.
#[derive(Clone, Default)]
pub(crate) struct RecoveryWriter {
    newest: std::sync::Arc<std::sync::Mutex<Option<u64>>>,
}

impl RecoveryWriter {
    /// Write `record` unless a newer generation is already on disk. Returns
    /// whether it wrote.
    pub(crate) fn save(
        &self,
        storage: &impl Storage,
        path: &Path,
        record: &RecoveryRecord,
        generation: u64,
    ) -> Result<bool, Failure> {
        let mut newest = self
            .newest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if newest.is_some_and(|newest| newest > generation) {
            return Ok(false);
        }
        save(storage, path, record)?;
        *newest = Some(generation);
        Ok(true)
    }

    /// Remove the draft at `path` if `generation` is still the newest write,
    /// as a stale autosave does when the document has changed or become
    /// clean since. Returns whether it removed.
    pub(crate) fn remove_if_newest(
        &self,
        storage: &impl Storage,
        path: &Path,
        generation: u64,
    ) -> Result<bool, Failure> {
        let newest = self
            .newest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *newest != Some(generation) {
            return Ok(false);
        }
        storage::remove_recovery(storage, path)?;
        Ok(true)
    }
}

#[cfg(not(unix))]
fn process_is_alive(_process_id: u32) -> bool {
    false
}

fn quarantine(path: &Path) {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let quarantine = path.with_file_name(format!("{name}.invalid"));
    if !quarantine.exists() {
        let _ = std::fs::rename(path, quarantine);
    }
}

fn encode(record: &RecoveryRecord) -> Result<Vec<u8>, &'static str> {
    if record.content.len() > MAX_DOCUMENT_BYTES {
        return Err("recovery content exceeds the 64 MiB safety limit");
    }
    let label = bounded_display_text(&record.document_label, MAX_LABEL_BYTES);
    let source = record
        .source_path
        .as_deref()
        .filter(|path| path.len() <= MAX_SOURCE_PATH_BYTES);
    let label_len = u16::try_from(label.len()).map_err(|_| "recovery label is too long")?;
    let source_len = source
        .map(str::len)
        .map(u32::try_from)
        .transpose()
        .map_err(|_| "recovery source path is too long")?
        .unwrap_or(u32::MAX);
    let content_len =
        u32::try_from(record.content.len()).map_err(|_| "recovery content is too long")?;
    let mut output = Vec::with_capacity(
        37_usize
            .saturating_add(label.len())
            .saturating_add(source.map(str::len).unwrap_or_default())
            .saturating_add(record.content.len()),
    );
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&record.created_unix_ms.to_le_bytes());
    output.push(encoding_code(record.format.encoding));
    output.push(line_ending_code(record.format.source_line_ending));
    output.push(line_ending_code(record.format.save_line_ending));
    output.extend_from_slice(&label_len.to_le_bytes());
    output.extend_from_slice(&source_len.to_le_bytes());
    output.extend_from_slice(&content_len.to_le_bytes());
    output.extend_from_slice(label.as_bytes());
    if let Some(source) = source {
        output.extend_from_slice(source.as_bytes());
    }
    output.extend_from_slice(record.content.as_bytes());
    if output.len() > MAX_RECORD_BYTES {
        return Err("recovery record exceeds its safety limit");
    }
    Ok(output)
}

fn decode(bytes: &[u8]) -> Result<RecoveryRecord, &'static str> {
    if bytes.len() > MAX_RECORD_BYTES || !bytes.starts_with(MAGIC) {
        return Err("recovery record has an invalid header");
    }
    let mut cursor = MAGIC.len();
    let created_unix_ms = take_u64(bytes, &mut cursor)?;
    let encoding = decode_encoding(take_byte(bytes, &mut cursor)?)?;
    let source_line_ending = decode_line_ending(take_byte(bytes, &mut cursor)?)?;
    let save_line_ending = decode_line_ending(take_byte(bytes, &mut cursor)?)?;
    if matches!(save_line_ending, LineEnding::None | LineEnding::Mixed) {
        return Err("recovery record has an invalid save line ending");
    }
    let label_len = usize::from(take_u16(bytes, &mut cursor)?);
    let source_len = take_u32(bytes, &mut cursor)?;
    let content_len = usize::try_from(take_u32(bytes, &mut cursor)?)
        .map_err(|_| "recovery content length is invalid")?;
    if label_len > MAX_LABEL_BYTES || content_len > MAX_DOCUMENT_BYTES {
        return Err("recovery record exceeds field bounds");
    }
    let label = take_utf8(bytes, &mut cursor, label_len)?;
    let source_path = if source_len == u32::MAX {
        None
    } else {
        let source_len =
            usize::try_from(source_len).map_err(|_| "recovery source length is invalid")?;
        if source_len > MAX_SOURCE_PATH_BYTES {
            return Err("recovery source path exceeds its bound");
        }
        Some(take_utf8(bytes, &mut cursor, source_len)?)
    };
    let content = take_utf8(bytes, &mut cursor, content_len)?;
    if cursor != bytes.len() {
        return Err("recovery record contains trailing data");
    }
    Ok(RecoveryRecord {
        created_unix_ms,
        document_label: bounded_display_text(&label, MAX_LABEL_BYTES),
        source_path,
        format: TextFormat {
            encoding,
            source_line_ending,
            save_line_ending,
        },
        content,
    })
}

fn take_byte(bytes: &[u8], cursor: &mut usize) -> Result<u8, &'static str> {
    let byte = *bytes
        .get(*cursor)
        .ok_or("recovery record ended unexpectedly")?;
    *cursor += 1;
    Ok(byte)
}

fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, &'static str> {
    let value = take_array::<2>(bytes, cursor)?;
    Ok(u16::from_le_bytes(value))
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, &'static str> {
    let value = take_array::<4>(bytes, cursor)?;
    Ok(u32::from_le_bytes(value))
}

fn take_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, &'static str> {
    let value = take_array::<8>(bytes, cursor)?;
    Ok(u64::from_le_bytes(value))
}

fn take_array<const SIZE: usize>(
    bytes: &[u8],
    cursor: &mut usize,
) -> Result<[u8; SIZE], &'static str> {
    let end = cursor
        .checked_add(SIZE)
        .ok_or("recovery record length overflowed")?;
    let value = bytes
        .get(*cursor..end)
        .ok_or("recovery record ended unexpectedly")?
        .try_into()
        .map_err(|_| "recovery record field is invalid")?;
    *cursor = end;
    Ok(value)
}

fn take_utf8(bytes: &[u8], cursor: &mut usize, length: usize) -> Result<String, &'static str> {
    let end = cursor
        .checked_add(length)
        .ok_or("recovery record length overflowed")?;
    let value = std::str::from_utf8(
        bytes
            .get(*cursor..end)
            .ok_or("recovery record ended unexpectedly")?,
    )
    .map_err(|_| "recovery record contains invalid UTF-8")?
    .to_string();
    *cursor = end;
    Ok(value)
}

fn encoding_code(encoding: TextEncoding) -> u8 {
    match encoding {
        TextEncoding::Utf8 => 0,
        TextEncoding::Utf8Bom => 1,
        TextEncoding::Utf16Le => 2,
        TextEncoding::Utf16Be => 3,
    }
}

fn decode_encoding(code: u8) -> Result<TextEncoding, &'static str> {
    match code {
        0 => Ok(TextEncoding::Utf8),
        1 => Ok(TextEncoding::Utf8Bom),
        2 => Ok(TextEncoding::Utf16Le),
        3 => Ok(TextEncoding::Utf16Be),
        _ => Err("recovery record has an unknown encoding"),
    }
}

fn line_ending_code(line_ending: LineEnding) -> u8 {
    match line_ending {
        LineEnding::None => 0,
        LineEnding::Lf => 1,
        LineEnding::CrLf => 2,
        LineEnding::Cr => 3,
        LineEnding::Mixed => 4,
    }
}

fn decode_line_ending(code: u8) -> Result<LineEnding, &'static str> {
    match code {
        0 => Ok(LineEnding::None),
        1 => Ok(LineEnding::Lf),
        2 => Ok(LineEnding::CrLf),
        3 => Ok(LineEnding::Cr),
        4 => Ok(LineEnding::Mixed),
        _ => Err("recovery record has an unknown line ending"),
    }
}

fn bounded_display_text(value: &str, maximum: usize) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(maximum);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    let value = normalized[..end].trim();
    if value.is_empty() {
        "Untitled".to_string()
    } else {
        value.to_string()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(content: &str) -> RecoveryRecord {
        RecoveryRecord {
            created_unix_ms: 42,
            document_label: "Draft.txt".into(),
            source_path: Some("/home/user/Draft.txt".into()),
            format: TextFormat {
                encoding: TextEncoding::Utf16Le,
                source_line_ending: LineEnding::Mixed,
                save_line_ending: LineEnding::CrLf,
            },
            content: content.into(),
        }
    }

    #[test]
    fn versioned_record_round_trips_format_identity_and_content() {
        let expected = record("hello 🦀\n");
        assert_eq!(decode(&encode(&expected).unwrap()).unwrap(), expected);
    }

    #[test]
    fn malformed_trailing_and_unknown_values_fail_closed() {
        assert!(decode(b"not a recovery record").is_err());
        let mut trailing = encode(&record("draft")).unwrap();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        let mut unknown_encoding = encode(&record("draft")).unwrap();
        unknown_encoding[MAGIC.len() + 8] = 99;
        assert!(decode(&unknown_encoding).is_err());
    }

    #[test]
    fn labels_are_control_free_and_record_paths_are_unique() {
        let record = RecoveryRecord::for_document(
            Some(Path::new("bad\nname.txt")),
            TextFormat::default(),
            "draft".into(),
        );
        assert_eq!(record.document_label, "bad name.txt");
        let directory = Path::new("recovery");
        let first = fresh_record_path(directory);
        let second = fresh_record_path(directory);
        assert_ne!(first, second);
        assert_eq!(
            active_owner(first.file_name().unwrap().to_str().unwrap()),
            Some(std::process::id())
        );
    }

    #[test]
    fn discovery_sorts_records_and_quarantines_malformed_files() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-editor-recovery-test-{}-{}",
            std::process::id(),
            RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let older = directory.join("draft-old.rmac-recovery");
        let newer = directory.join("draft-new.rmac-recovery");
        let malformed = directory.join("draft-bad.rmac-recovery");
        let mut old_record = record("old");
        old_record.created_unix_ms = 1;
        let mut new_record = record("new");
        new_record.created_unix_ms = 2;
        save(&rmac_storage::FileSystem, &older, &old_record).unwrap();
        save(&rmac_storage::FileSystem, &newer, &new_record).unwrap();
        std::fs::write(&malformed, b"bad").unwrap();

        let discovery = discover(&directory);
        assert_eq!(discovery.candidates.len(), 2);
        assert_eq!(discovery.candidates[0].record.content, "new");
        assert_eq!(discovery.malformed, 1);
        assert!(directory.join("draft-bad.rmac-recovery.invalid").is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn one_orphan_record_can_be_claimed_only_once() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-editor-recovery-claim-test-{}-{}",
            std::process::id(),
            RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("draft-orphan.rmac-recovery");
        let candidate = Candidate {
            path: path.clone(),
            record: record("draft"),
        };
        save(&rmac_storage::FileSystem, &path, &candidate.record).unwrap();

        let claimed = claim(&directory, candidate.clone()).unwrap();
        assert!(claimed.path.is_file());
        assert_eq!(
            active_owner(claimed.path.file_name().unwrap().to_str().unwrap()),
            Some(std::process::id())
        );
        assert!(claim(&directory, candidate).is_err());
        assert!(discover(&directory).candidates.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn scratch_directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rmac-editor-{label}-{}-{}",
            std::process::id(),
            RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn an_older_autosave_never_replaces_or_removes_a_newer_flush() {
        let directory = scratch_directory("recovery-writer");
        let path = fresh_record_path(&directory);
        let writer = RecoveryWriter::default();
        let storage = rmac_storage::FileSystem;

        // The session-end flush writes generation 7 first...
        assert!(writer.save(&storage, &path, &record("newest"), 7).unwrap());
        // ...then the autosave that read generation 5 finishes late.
        assert!(!writer.save(&storage, &path, &record("older"), 5).unwrap());
        assert!(!writer.remove_if_newest(&storage, &path, 5).unwrap());
        let decoded = decode(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded.content, "newest");

        // The newest writer can still remove its own draft.
        assert!(writer.remove_if_newest(&storage, &path, 7).unwrap());
        assert!(!path.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_draft_flushed_before_a_forced_exit_is_offered_on_the_next_launch() {
        let directory = scratch_directory("recovery-forced-exit");
        // What the flush leaves behind: a record named for its owner, which
        // has since gone. Process 0 is never a live owner.
        let flushed = directory.join(format!(
            "{RECORD_PREFIX}1-0{ACTIVE_OWNER_MARKER}0{RECORD_SUFFIX}"
        ));
        let writer = RecoveryWriter::default();
        let mut draft = RecoveryRecord::for_document(
            Some(Path::new("/home/user/Letter.txt")),
            TextFormat::default(),
            "typed just before the power went".into(),
        );
        draft.created_unix_ms = 99;
        writer
            .save(&rmac_storage::FileSystem, &flushed, &draft, 3)
            .unwrap();

        let discovery = discover(&directory);
        assert_eq!(discovery.candidates.len(), 1);
        let claimed = claim(&directory, discovery.candidates[0].clone()).unwrap();
        assert_eq!(claimed.record, draft);
        assert_eq!(claimed.record.document_label, "Letter.txt");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_reused_process_id_does_not_hide_a_draft() {
        let editor: &[u8] = b"rmac-text-edito\n";
        assert!(owner_runs_same_program(Some(editor), Some(editor)));
        // After a reboot the number may belong to anything else.
        assert!(!owner_runs_same_program(Some(editor), Some(b"systemd\n")));
        // The owner exited between the two checks.
        assert!(!owner_runs_same_program(Some(editor), None));
        // Without our own name, stay on the safe side of a live editor.
        assert!(owner_runs_same_program(None, Some(b"anything\n")));
    }

    #[test]
    fn discovery_skips_live_records_but_recovers_dead_owner_records() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-editor-recovery-owner-test-{}-{}",
            std::process::id(),
            RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let live = fresh_record_path(&directory);
        let dead = directory.join(format!(
            "{RECORD_PREFIX}dead{ACTIVE_OWNER_MARKER}0{RECORD_SUFFIX}"
        ));
        save(&rmac_storage::FileSystem, &live, &record("live")).unwrap();
        save(&rmac_storage::FileSystem, &dead, &record("dead")).unwrap();

        let discovery = discover(&directory);

        assert_eq!(discovery.candidates.len(), 1);
        assert_eq!(discovery.candidates[0].record.content, "dead");
        assert!(live.is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
