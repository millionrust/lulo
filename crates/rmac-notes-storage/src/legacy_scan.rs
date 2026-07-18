use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Read as _};
use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_notes_store::{
    SortOrder, MAX_ATTACHMENTS, MAX_ATTACHMENT_BYTES, MAX_LIBRARY_BYTES, MAX_NOTES,
};

use crate::migration::{
    parse_note_path, validate_attachment_path, LegacyAttachmentInput, LegacyLibraryInput,
    LegacyNoteInput, MAX_TOTAL_ATTACHMENT_BYTES,
};

const MAX_PIN_BYTES: usize = MAX_LIBRARY_BYTES;
const MAX_SORT_BYTES: usize = 64;
const MAX_LEGACY_ENTRIES: usize = MAX_NOTES + MAX_ATTACHMENTS + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyScanOperation {
    ValidateRoot,
    ReadDirectory,
    InspectEntry,
    ReadNote,
    ReadAttachment,
    ReadPins,
    ReadSort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyScanErrorKind {
    Io(io::ErrorKind),
    Symlink,
    UnsupportedEntry,
    InvalidUtf8,
    InvalidMetadata,
    InvalidTimestamp,
    CollectionLimit,
    SourceTooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyScanError {
    pub operation: LegacyScanOperation,
    pub kind: LegacyScanErrorKind,
}

impl LegacyScanError {
    fn new(operation: LegacyScanOperation, kind: LegacyScanErrorKind) -> Self {
        Self { operation, kind }
    }

    fn io(operation: LegacyScanOperation, error: io::Error) -> Self {
        Self::new(operation, LegacyScanErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for LegacyScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            LegacyScanErrorKind::Io(_) => "Notes could not read the legacy library",
            LegacyScanErrorKind::Symlink => {
                "The legacy Notes library contains a symbolic link that needs review"
            }
            LegacyScanErrorKind::UnsupportedEntry => {
                "The legacy Notes library contains an unsupported filesystem entry"
            }
            LegacyScanErrorKind::InvalidUtf8 | LegacyScanErrorKind::InvalidMetadata => {
                "The legacy Notes library contains invalid metadata"
            }
            LegacyScanErrorKind::InvalidTimestamp => {
                "The legacy Notes library contains an invalid timestamp"
            }
            LegacyScanErrorKind::CollectionLimit | LegacyScanErrorKind::SourceTooLarge => {
                "The legacy Notes library exceeds a migration safety limit"
            }
        })
    }
}

impl std::error::Error for LegacyScanError {}

/// Deterministically reread the complete path-based Notes prototype.
///
/// The source root must be absolute and may not itself be a symlink. Entries
/// are sorted by validated relative path, regular files are opened no-follow
/// on Unix and bounded before allocation, and no source path is modified.
pub fn scan_legacy_library(root: &Path) -> Result<LegacyLibraryInput, LegacyScanError> {
    if !root.is_absolute() {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ValidateRoot,
            LegacyScanErrorKind::InvalidMetadata,
        ));
    }
    let root_metadata = std::fs::symlink_metadata(root)
        .map_err(|error| LegacyScanError::io(LegacyScanOperation::ValidateRoot, error))?;
    if root_metadata.file_type().is_symlink() {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ValidateRoot,
            LegacyScanErrorKind::Symlink,
        ));
    }
    if !root_metadata.is_dir() {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ValidateRoot,
            LegacyScanErrorKind::UnsupportedEntry,
        ));
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| LegacyScanError::io(LegacyScanOperation::ValidateRoot, error))?;

    let mut notes = Vec::new();
    let mut attachments = Vec::new();
    let mut folder_names = Vec::new();
    let mut pins = Vec::new();
    let mut sort_order = SortOrder::Edited;
    let mut note_bytes = 0_usize;
    let mut attachment_bytes = 0_u64;
    let mut root_entries = directory_entries(root)?;
    root_entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in root_entries {
        let name = entry_name(&entry)?;
        let file_type = entry
            .file_type()
            .map_err(|error| LegacyScanError::io(LegacyScanOperation::InspectEntry, error))?;
        if file_type.is_symlink() {
            return Err(LegacyScanError::new(
                LegacyScanOperation::InspectEntry,
                LegacyScanErrorKind::Symlink,
            ));
        }
        if file_type.is_dir() {
            folder_names.push(name.clone());
            if folder_names.len() > rmac_notes_store::MAX_FOLDERS {
                return Err(LegacyScanError::new(
                    LegacyScanOperation::ReadDirectory,
                    LegacyScanErrorKind::CollectionLimit,
                ));
            }
            scan_folder(
                &entry.path(),
                &name,
                &mut notes,
                &mut note_bytes,
                &mut attachments,
                &mut attachment_bytes,
            )?;
        } else if file_type.is_file() {
            match name.as_str() {
                ".pinned" => {
                    let (bytes, _) = read_regular_file(
                        &entry.path(),
                        MAX_PIN_BYTES,
                        LegacyScanOperation::ReadPins,
                    )?;
                    pins = parse_pins(&bytes, root, &canonical_root)?;
                }
                ".sort" => {
                    let (bytes, _) = read_regular_file(
                        &entry.path(),
                        MAX_SORT_BYTES,
                        LegacyScanOperation::ReadSort,
                    )?;
                    sort_order = parse_sort(&bytes)?;
                }
                _ if name.ends_with(".md") => {
                    push_note(&mut notes, &mut note_bytes, &entry.path(), name)?;
                }
                _ => push_attachment(&mut attachments, &mut attachment_bytes, &entry.path(), name)?,
            }
        } else {
            return Err(LegacyScanError::new(
                LegacyScanOperation::InspectEntry,
                LegacyScanErrorKind::UnsupportedEntry,
            ));
        }
    }
    notes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    attachments.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    folder_names.sort();
    pins.sort();
    pins.dedup();
    if notes.len() > MAX_NOTES || attachments.len() > MAX_ATTACHMENTS || pins.len() > MAX_NOTES {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ReadDirectory,
            LegacyScanErrorKind::CollectionLimit,
        ));
    }
    Ok(LegacyLibraryInput {
        folder_names,
        notes,
        attachments,
        pinned_note_paths: pins,
        sort_order,
    })
}

fn scan_folder(
    path: &Path,
    folder_name: &str,
    notes: &mut Vec<LegacyNoteInput>,
    note_bytes: &mut usize,
    attachments: &mut Vec<LegacyAttachmentInput>,
    attachment_bytes: &mut u64,
) -> Result<(), LegacyScanError> {
    let mut entries = directory_entries(path)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry_name(&entry)?;
        let file_type = entry
            .file_type()
            .map_err(|error| LegacyScanError::io(LegacyScanOperation::InspectEntry, error))?;
        if file_type.is_symlink() {
            return Err(LegacyScanError::new(
                LegacyScanOperation::InspectEntry,
                LegacyScanErrorKind::Symlink,
            ));
        }
        if !file_type.is_file() {
            return Err(LegacyScanError::new(
                LegacyScanOperation::InspectEntry,
                LegacyScanErrorKind::UnsupportedEntry,
            ));
        }
        let relative_path = format!("{folder_name}/{name}");
        if name.ends_with(".md") {
            push_note(notes, note_bytes, &entry.path(), relative_path)?;
        } else {
            push_attachment(attachments, attachment_bytes, &entry.path(), relative_path)?;
        }
    }
    Ok(())
}

fn directory_entries(path: &Path) -> Result<Vec<std::fs::DirEntry>, LegacyScanError> {
    let directory = std::fs::read_dir(path)
        .map_err(|error| LegacyScanError::io(LegacyScanOperation::ReadDirectory, error))?;
    let mut entries = Vec::new();
    for entry in directory {
        entries.push(
            entry
                .map_err(|error| LegacyScanError::io(LegacyScanOperation::ReadDirectory, error))?,
        );
        if entries.len() > MAX_LEGACY_ENTRIES {
            return Err(LegacyScanError::new(
                LegacyScanOperation::ReadDirectory,
                LegacyScanErrorKind::CollectionLimit,
            ));
        }
    }
    Ok(entries)
}

fn entry_name(entry: &std::fs::DirEntry) -> Result<String, LegacyScanError> {
    entry.file_name().into_string().map_err(|_| {
        LegacyScanError::new(
            LegacyScanOperation::InspectEntry,
            LegacyScanErrorKind::InvalidUtf8,
        )
    })
}

fn read_note(path: &Path, relative_path: String) -> Result<LegacyNoteInput, LegacyScanError> {
    parse_note_path(&relative_path).map_err(|_| {
        LegacyScanError::new(
            LegacyScanOperation::ReadNote,
            LegacyScanErrorKind::InvalidMetadata,
        )
    })?;
    let (bytes, metadata) =
        read_regular_file(path, MAX_LIBRARY_BYTES, LegacyScanOperation::ReadNote)?;
    let modified = metadata
        .modified()
        .map_err(|error| LegacyScanError::io(LegacyScanOperation::ReadNote, error))?;
    let created = metadata.created().unwrap_or(modified);
    let created_unix_ms = unix_millis(created)?;
    let modified_unix_ms = unix_millis(modified)?;
    if modified_unix_ms < created_unix_ms {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ReadNote,
            LegacyScanErrorKind::InvalidTimestamp,
        ));
    }
    Ok(LegacyNoteInput {
        relative_path,
        bytes,
        created_unix_ms,
        modified_unix_ms,
    })
}

fn push_note(
    notes: &mut Vec<LegacyNoteInput>,
    total_bytes: &mut usize,
    path: &Path,
    relative_path: String,
) -> Result<(), LegacyScanError> {
    if notes.len() >= MAX_NOTES {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ReadNote,
            LegacyScanErrorKind::CollectionLimit,
        ));
    }
    let note = read_note(path, relative_path)?;
    *total_bytes = total_bytes
        .checked_add(note.bytes.len())
        .filter(|total| *total <= MAX_LIBRARY_BYTES)
        .ok_or_else(|| {
            LegacyScanError::new(
                LegacyScanOperation::ReadNote,
                LegacyScanErrorKind::SourceTooLarge,
            )
        })?;
    notes.push(note);
    Ok(())
}

fn push_attachment(
    attachments: &mut Vec<LegacyAttachmentInput>,
    total_bytes: &mut u64,
    path: &Path,
    relative_path: String,
) -> Result<(), LegacyScanError> {
    if attachments.len() >= MAX_ATTACHMENTS {
        return Err(LegacyScanError::new(
            LegacyScanOperation::ReadAttachment,
            LegacyScanErrorKind::CollectionLimit,
        ));
    }
    validate_attachment_path(&relative_path).map_err(|_| {
        LegacyScanError::new(
            LegacyScanOperation::ReadAttachment,
            LegacyScanErrorKind::InvalidMetadata,
        )
    })?;
    let (bytes, _) = read_regular_file(
        path,
        usize::try_from(MAX_ATTACHMENT_BYTES).unwrap_or(usize::MAX),
        LegacyScanOperation::ReadAttachment,
    )?;
    *total_bytes = total_bytes
        .checked_add(bytes.len() as u64)
        .filter(|total| *total <= MAX_TOTAL_ATTACHMENT_BYTES)
        .ok_or_else(|| {
            LegacyScanError::new(
                LegacyScanOperation::ReadAttachment,
                LegacyScanErrorKind::SourceTooLarge,
            )
        })?;
    attachments.push(LegacyAttachmentInput {
        relative_path,
        bytes,
    });
    Ok(())
}

fn read_regular_file(
    path: &Path,
    maximum: usize,
    operation: LegacyScanOperation,
) -> Result<(Vec<u8>, Metadata), LegacyScanError> {
    let file = open_no_follow(path).map_err(|error| LegacyScanError::io(operation, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| LegacyScanError::io(operation, error))?;
    if !metadata.is_file() {
        return Err(LegacyScanError::new(
            operation,
            LegacyScanErrorKind::UnsupportedEntry,
        ));
    }
    if metadata.len() > maximum as u64 {
        return Err(LegacyScanError::new(
            operation,
            LegacyScanErrorKind::SourceTooLarge,
        ));
    }
    let mut bytes = Vec::with_capacity(maximum.min(metadata.len() as usize));
    file.take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| LegacyScanError::io(operation, error))?;
    if bytes.len() > maximum {
        return Err(LegacyScanError::new(
            operation,
            LegacyScanErrorKind::SourceTooLarge,
        ));
    }
    Ok((bytes, metadata))
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

fn parse_pins(
    bytes: &[u8],
    root: &Path,
    canonical_root: &Path,
) -> Result<Vec<String>, LegacyScanError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        LegacyScanError::new(
            LegacyScanOperation::ReadPins,
            LegacyScanErrorKind::InvalidUtf8,
        )
    })?;
    let mut pins = Vec::new();
    for raw in text.lines() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let path = Path::new(raw);
        let relative = if path.is_absolute() {
            path.strip_prefix(root)
                .or_else(|_| path.strip_prefix(canonical_root))
                .map_err(|_| invalid_pin())?
        } else {
            path
        };
        let relative = portable_relative_path(relative).ok_or_else(invalid_pin)?;
        parse_note_path(&relative).map_err(|_| invalid_pin())?;
        pins.push(relative);
        if pins.len() > MAX_NOTES {
            return Err(LegacyScanError::new(
                LegacyScanOperation::ReadPins,
                LegacyScanErrorKind::CollectionLimit,
            ));
        }
    }
    Ok(pins)
}

fn portable_relative_path(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_str()?);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn invalid_pin() -> LegacyScanError {
    LegacyScanError::new(
        LegacyScanOperation::ReadPins,
        LegacyScanErrorKind::InvalidMetadata,
    )
}

fn parse_sort(bytes: &[u8]) -> Result<SortOrder, LegacyScanError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| {
            LegacyScanError::new(
                LegacyScanOperation::ReadSort,
                LegacyScanErrorKind::InvalidUtf8,
            )
        })?
        .trim();
    match value {
        "edited" => Ok(SortOrder::Edited),
        "created" => Ok(SortOrder::Created),
        "title" => Ok(SortOrder::Title),
        _ => Err(LegacyScanError::new(
            LegacyScanOperation::ReadSort,
            LegacyScanErrorKind::InvalidMetadata,
        )),
    }
}

fn unix_millis(time: SystemTime) -> Result<u64, LegacyScanError> {
    let millis = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            LegacyScanError::new(
                LegacyScanOperation::ReadNote,
                LegacyScanErrorKind::InvalidTimestamp,
            )
        })?
        .as_millis();
    u64::try_from(millis).map_err(|_| {
        LegacyScanError::new(
            LegacyScanOperation::ReadNote,
            LegacyScanErrorKind::InvalidTimestamp,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_legacy_library;
    use image::ImageEncoder as _;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-notes-legacy-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn fixture() -> PathBuf {
        let root = temp_root("fixture");
        std::fs::create_dir_all(root.join("Projects")).unwrap();
        std::fs::create_dir(root.join("Empty")).unwrap();
        std::fs::write(
            root.join("root.md"),
            b"Root\n![](diagram.png)\n<!--tags: rmac-->",
        )
        .unwrap();
        std::fs::write(root.join("Projects/nested.md"), b"Nested\nBody").unwrap();
        std::fs::write(root.join("diagram.png"), png()).unwrap();
        std::fs::write(root.join("Projects/unclaimed.bin"), b"keep nested").unwrap();
        std::fs::write(
            root.join(".pinned"),
            root.join("Projects/nested.md").to_string_lossy().as_bytes(),
        )
        .unwrap();
        std::fs::write(root.join(".sort"), b"title").unwrap();
        root
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    #[test]
    fn complete_scan_is_deterministic_and_consumable_by_the_planner() {
        let root = fixture();

        let first = scan_legacy_library(&root).unwrap();
        let second = scan_legacy_library(&root).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.sort_order, SortOrder::Title);
        assert_eq!(
            first
                .notes
                .iter()
                .map(|note| note.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["Projects/nested.md", "root.md"]
        );
        assert_eq!(first.pinned_note_paths, ["Projects/nested.md"]);
        assert!(first
            .attachments
            .iter()
            .any(|file| file.relative_path == "Projects/unclaimed.bin"));
        let plan = plan_legacy_library(first).unwrap();
        assert_eq!(plan.snapshot.notes.len(), 2);
        assert!(plan
            .snapshot
            .folders
            .iter()
            .any(|folder| folder.name == "Empty"));
        assert_eq!(plan.snapshot.attachments.len(), 1);
        assert!(plan
            .recovery_files
            .iter()
            .any(|file| file.relative_path == "Projects/unclaimed.bin"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn source_and_entry_symlinks_fail_closed() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        symlink(root.join("diagram.png"), root.join("linked.png")).unwrap();
        let error = scan_legacy_library(&root).unwrap_err();
        assert_eq!(error.kind, LegacyScanErrorKind::Symlink);
        std::fs::remove_file(root.join("linked.png")).unwrap();

        let alias = temp_root("root-alias");
        symlink(&root, &alias).unwrap();
        let error = scan_legacy_library(&alias).unwrap_err();
        assert_eq!(error.kind, LegacyScanErrorKind::Symlink);
        std::fs::remove_file(alias).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pins_outside_the_source_and_invalid_sort_never_silently_default() {
        let root = fixture();
        std::fs::write(root.join(".pinned"), b"/outside/note.md").unwrap();
        let pin_error = scan_legacy_library(&root).unwrap_err();
        assert_eq!(pin_error.operation, LegacyScanOperation::ReadPins);
        assert_eq!(pin_error.kind, LegacyScanErrorKind::InvalidMetadata);

        std::fs::write(root.join(".pinned"), b"").unwrap();
        std::fs::write(root.join(".sort"), b"mystery").unwrap();
        let sort_error = scan_legacy_library(&root).unwrap_err();
        assert_eq!(sort_error.operation, LegacyScanOperation::ReadSort);
        assert_eq!(sort_error.kind, LegacyScanErrorKind::InvalidMetadata);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn excessive_files_are_rejected_by_metadata_before_allocation() {
        let root = temp_root("oversized");
        std::fs::create_dir(&root).unwrap();
        let oversized = root.join("oversized.bin");
        File::create(&oversized)
            .unwrap()
            .set_len(MAX_ATTACHMENT_BYTES + 1)
            .unwrap();

        let error = scan_legacy_library(&root).unwrap_err();

        assert_eq!(error.operation, LegacyScanOperation::ReadAttachment);
        assert_eq!(error.kind, LegacyScanErrorKind::SourceTooLarge);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deeper_directory_trees_are_not_partially_imported() {
        let root = fixture();
        std::fs::create_dir(root.join("Projects/deeper")).unwrap();

        let error = scan_legacy_library(&root).unwrap_err();

        assert_eq!(error.kind, LegacyScanErrorKind::UnsupportedEntry);
        std::fs::remove_dir_all(root).unwrap();
    }
}
