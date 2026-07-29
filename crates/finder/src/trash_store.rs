use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::operation_journal::{EntryIdentity, TreeManifest, TreeSnapshot as TransferTreeSnapshot};
use crate::undo_journal::{TrashUndoSeed, UndoAvailability, UndoKind, UndoOutcome, UndoStore};

const RECORD_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_RECORDS: usize = 256;
const MAX_INFO_BYTES: u64 = 16 * 1024;
const MAX_FILENAME_BYTES: usize = 255;
const TRASHINFO_SUFFIX: &str = ".trashinfo";
const MAX_TRASH_ITEMS: usize = 100_000;
const MAX_TRASH_PATH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TrashOperation {
    #[default]
    Trash,
    Restore,
    Delete,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TrashStage {
    Prepared,
    InfoPublished,
    DataMoved,
    RestorePrepared,
    RestoreDataMoved,
    RestoreInfoRemoved,
    DeletePrepared,
    DeleteDataStaged,
    DeleteDataRemoved,
    DeleteInfoRemoved,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TrashResolutionIntent {
    ReturnDeleteStage,
    RebuildMetadata,
    ReturnDeleteStageAndRebuildMetadata,
    PreserveConflictingItems,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct TrashRecord {
    version: u32,
    id: String,
    #[serde(default)]
    operation: TrashOperation,
    stage: TrashStage,
    source_path_bytes: Vec<u8>,
    trash_root_path_bytes: Vec<u8>,
    data_path_bytes: Vec<u8>,
    info_path_bytes: Vec<u8>,
    source_identity: EntryIdentity,
    source_manifest: TreeManifest,
    info_sha256: [u8; 32],
    info_identity: Option<EntryIdentity>,
    #[serde(default)]
    restore_parent_identity: Option<EntryIdentity>,
    #[serde(default)]
    delete_path_bytes: Option<Vec<u8>>,
    #[serde(default)]
    resolution_intent: Option<TrashResolutionIntent>,
    #[serde(default)]
    resolution_identity: Option<EntryIdentity>,
    #[serde(default)]
    resolution_manifest: Option<TreeManifest>,
    #[serde(default)]
    resolution_info_identity: Option<EntryIdentity>,
    #[serde(default)]
    resolution_info_sha256: Option<[u8; 32]>,
    #[serde(default)]
    resolution_info_bytes: Option<Vec<u8>>,
    #[serde(default)]
    resolution_primary_identity: Option<EntryIdentity>,
    #[serde(default)]
    resolution_primary_manifest: Option<TreeManifest>,
    #[serde(default)]
    resolution_primary_info_identity: Option<EntryIdentity>,
    #[serde(default)]
    resolution_data_path_bytes: Option<Vec<u8>>,
    #[serde(default)]
    resolution_info_path_bytes: Option<Vec<u8>>,
    #[serde(default)]
    create_undo_receipt: bool,
    #[serde(default)]
    trash_source_parent_identity: Option<EntryIdentity>,
}

impl TrashRecord {
    fn source(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.source_path_bytes.clone()))
    }

    fn trash_root(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.trash_root_path_bytes.clone()))
    }

    fn data_path(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.data_path_bytes.clone()))
    }

    fn info_path(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.info_path_bytes.clone()))
    }

    fn delete_path(&self) -> Option<PathBuf> {
        self.delete_path_bytes
            .as_ref()
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes.clone())))
    }

    fn resolution_data_path(&self) -> Option<PathBuf> {
        self.resolution_data_path_bytes
            .as_ref()
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes.clone())))
    }

    fn resolution_info_path(&self) -> Option<PathBuf> {
        self.resolution_info_path_bytes
            .as_ref()
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes.clone())))
    }

    fn validate(&self, expected_id: &str) -> io::Result<()> {
        if self.version != RECORD_VERSION
            || self.id != expected_id
            || Uuid::parse_str(&self.id).is_err()
        {
            return Err(invalid_data("trash transaction identity is invalid"));
        }
        let source = self.source();
        let root = self.trash_root();
        let data = self.data_path();
        let info = self.info_path();
        let delete = self.delete_path();
        let files = root.join("files");
        let info_directory = root.join("info");
        if !path_is_normal_absolute(&source)
            || !path_is_normal_absolute(&root)
            || !path_is_normal_absolute(&data)
            || !path_is_normal_absolute(&info)
            || data.parent() != Some(files.as_path())
            || info.parent() != Some(info_directory.as_path())
            || source == data
            || source.starts_with(&root)
            || root.starts_with(&source)
            || !trash_root_is_structurally_valid(&root)
        {
            return Err(invalid_data("trash transaction paths are invalid"));
        }
        let data_name = data
            .file_name()
            .ok_or_else(|| invalid_data("trash data name is missing"))?;
        let info_name = info
            .file_name()
            .ok_or_else(|| invalid_data("trash info name is missing"))?;
        let mut expected_info = data_name.to_os_string();
        expected_info.push(TRASHINFO_SUFFIX);
        if info_name != expected_info {
            return Err(invalid_data("trash data and info names do not match"));
        }
        self.validate_resolution_state()?;
        match self.operation {
            TrashOperation::Trash => {
                if !matches!(
                    self.stage,
                    TrashStage::Prepared | TrashStage::InfoPublished | TrashStage::DataMoved
                ) || self.restore_parent_identity.is_some()
                    || delete.is_some()
                {
                    return Err(invalid_data("trash transaction stage is invalid"));
                }
                if self.create_undo_receipt != self.trash_source_parent_identity.is_some() {
                    return Err(invalid_data(
                        "Trash Undo parent evidence is missing or unexpected",
                    ));
                }
                if self.stage == TrashStage::Prepared && self.info_identity.is_some() {
                    return Err(invalid_data(
                        "prepared trash transaction has an info identity",
                    ));
                }
                if self.stage != TrashStage::Prepared && self.info_identity.is_none() {
                    return Err(invalid_data(
                        "advanced trash transaction is missing its info identity",
                    ));
                }
            }
            TrashOperation::Restore => {
                if !matches!(
                    self.stage,
                    TrashStage::RestorePrepared
                        | TrashStage::RestoreDataMoved
                        | TrashStage::RestoreInfoRemoved
                ) || self.info_identity.is_none()
                    || self.restore_parent_identity.is_none()
                    || delete.is_some()
                    || self.trash_source_parent_identity.is_some()
                {
                    return Err(invalid_data("restore transaction stage is invalid"));
                }
            }
            TrashOperation::Delete => {
                let delete = delete
                    .as_ref()
                    .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
                let expected_name = OsString::from(format!(".rmac-delete-{}", self.id));
                if !matches!(
                    self.stage,
                    TrashStage::DeletePrepared
                        | TrashStage::DeleteDataStaged
                        | TrashStage::DeleteDataRemoved
                        | TrashStage::DeleteInfoRemoved
                ) || self.info_identity.is_none()
                    || self.restore_parent_identity.is_some()
                    || !path_is_normal_absolute(delete)
                    || delete.parent() != Some(files.as_path())
                    || delete.file_name() != Some(expected_name.as_os_str())
                    || delete == &data
                    || delete == &info
                    || self.create_undo_receipt
                    || self.trash_source_parent_identity.is_some()
                {
                    return Err(invalid_data("delete transaction stage is invalid"));
                }
            }
        }
        Ok(())
    }

    fn validate_resolution_state(&self) -> io::Result<()> {
        match self.resolution_intent {
            None if !self.has_resolution_state() => Ok(()),
            Some(TrashResolutionIntent::ReturnDeleteStage)
                if self.operation == TrashOperation::Delete
                    && self.stage == TrashStage::DeleteDataStaged
                    && self.resolution_identity.is_some()
                    && self.resolution_manifest.is_some()
                    && self.resolution_info_identity.is_some()
                    && self.resolution_info_sha256.is_some()
                    && self.resolution_info_bytes.is_none()
                    && !self.has_conflict_resolution_state() =>
            {
                Ok(())
            }
            Some(TrashResolutionIntent::RebuildMetadata)
                if self.resolution_identity.is_some()
                    && self.resolution_manifest.is_some()
                    && self.resolution_info_sha256.is_some()
                    && self.resolution_info_bytes.as_ref().is_some_and(|bytes| {
                        bytes.len() as u64 <= MAX_INFO_BYTES
                            && Some(sha256(bytes)) == self.resolution_info_sha256
                    })
                    && !self.has_conflict_resolution_state() =>
            {
                Ok(())
            }
            Some(TrashResolutionIntent::ReturnDeleteStageAndRebuildMetadata)
                if self.operation == TrashOperation::Delete
                    && self.stage == TrashStage::DeleteDataStaged
                    && self.resolution_identity.is_some()
                    && self.resolution_manifest.is_some()
                    && self.resolution_info_sha256.is_some()
                    && self.resolution_info_bytes.as_ref().is_some_and(|bytes| {
                        bytes.len() as u64 <= MAX_INFO_BYTES
                            && Some(sha256(bytes)) == self.resolution_info_sha256
                    })
                    && !self.has_conflict_resolution_state() =>
            {
                Ok(())
            }
            Some(TrashResolutionIntent::PreserveConflictingItems)
                if self.operation == TrashOperation::Delete
                    && self.stage == TrashStage::DeleteDataStaged
                    && self.resolution_identity.is_some()
                    && self.resolution_manifest.is_some()
                    && self.resolution_primary_identity.is_some()
                    && self.resolution_primary_manifest.is_some()
                    && self.resolution_data_path().is_some_and(|path| {
                        path_is_normal_absolute(&path)
                            && path.parent() == Some(self.trash_root().join("files").as_path())
                            && path != self.data_path()
                            && self.delete_path().as_ref() != Some(&path)
                    })
                    && self.resolution_info_path().is_some_and(|path| {
                        let Some(data_name) = self
                            .resolution_data_path()
                            .and_then(|data| data.file_name().map(OsStr::to_os_string))
                        else {
                            return false;
                        };
                        let mut expected = data_name;
                        expected.push(TRASHINFO_SUFFIX);
                        path_is_normal_absolute(&path)
                            && path.parent() == Some(self.trash_root().join("info").as_path())
                            && path.file_name() == Some(expected.as_os_str())
                            && path != self.info_path()
                    })
                    && self.resolution_info_sha256.is_some()
                    && self.resolution_info_bytes.as_ref().is_some_and(|bytes| {
                        bytes.len() as u64 <= MAX_INFO_BYTES
                            && Some(sha256(bytes)) == self.resolution_info_sha256
                    }) =>
            {
                Ok(())
            }
            _ => Err(invalid_data(
                "Trash transaction resolution state is invalid",
            )),
        }
    }

    fn has_resolution_state(&self) -> bool {
        self.resolution_intent.is_some()
            || self.resolution_identity.is_some()
            || self.resolution_manifest.is_some()
            || self.resolution_info_identity.is_some()
            || self.resolution_info_sha256.is_some()
            || self.resolution_info_bytes.is_some()
            || self.resolution_primary_identity.is_some()
            || self.resolution_primary_manifest.is_some()
            || self.resolution_primary_info_identity.is_some()
            || self.resolution_data_path_bytes.is_some()
            || self.resolution_info_path_bytes.is_some()
    }

    fn has_conflict_resolution_state(&self) -> bool {
        self.resolution_primary_identity.is_some()
            || self.resolution_primary_manifest.is_some()
            || self.resolution_primary_info_identity.is_some()
            || self.resolution_data_path_bytes.is_some()
            || self.resolution_info_path_bytes.is_some()
    }
}

#[cfg(any(target_os = "linux", test))]
fn default_state_root() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "HOME is unavailable for Trash recovery",
                )
            })?;
            PathBuf::from(home).join(".local").join("state")
        }
    };
    if !base.is_absolute() {
        return Err(invalid_input(
            "Trash recovery state directory must be absolute",
        ));
    }
    Ok(base.join("rmac-files"))
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(invalid_input("private directory must be absolute"));
    }
    create_directories_durably(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid_data("private path is not a real directory"));
    }
    if metadata.uid() != effective_uid() {
        return Err(invalid_data(
            "private directory is not owned by the current user",
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    sync_directory(path)
}

fn create_directories_durably(path: &Path) -> io::Result<()> {
    let mut current = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(current) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(invalid_data("directory ancestor is not a real directory"));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                current = current
                    .parent()
                    .ok_or_else(|| invalid_input("directory has no existing ancestor"))?;
            }
            Err(error) => return Err(error),
        }
    }
    for directory in missing.iter().rev() {
        match fs::create_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid_data("created path is not a real directory"));
        }
        sync_directory(
            directory
                .parent()
                .ok_or_else(|| invalid_data("created directory has no parent"))?,
        )?;
    }
    Ok(())
}

fn ensure_trash_layout(layout: &TrashLayout) -> io::Result<()> {
    if !path_is_normal_absolute(&layout.topdir)
        || !path_is_normal_absolute(&layout.root)
        || !layout.root.starts_with(&layout.topdir)
    {
        return Err(invalid_input("Trash layout paths are invalid"));
    }
    ensure_private_directory(&layout.root)?;
    ensure_private_directory(&layout.root.join("files"))?;
    ensure_private_directory(&layout.root.join("info"))?;
    let root_device = fs::symlink_metadata(&layout.root)?.dev();
    let topdir_device = fs::symlink_metadata(&layout.topdir)?.dev();
    if root_device != topdir_device {
        return Err(changed("Trash directory moved to another filesystem"));
    }
    sync_directory(&layout.root)
}

fn validate_source_path(source: &Path) -> io::Result<()> {
    if !path_is_normal_absolute(source) || source == Path::new("/") || source.file_name().is_none()
    {
        return Err(invalid_input("Trash source path is invalid"));
    }
    let parent = source
        .parent()
        .ok_or_else(|| invalid_input("Trash source has no parent"))?;
    let canonical_parent = fs::canonicalize(parent)?;
    if canonical_parent != parent {
        return Err(invalid_input(
            "Trash source parent must be a canonical directory",
        ));
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || metadata.is_file() || metadata.is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "special files cannot be moved to Trash",
        ))
    }
}

fn canonical_source_path(source: &Path) -> io::Result<PathBuf> {
    if !path_is_normal_absolute(source) || source.file_name().is_none() {
        return Err(invalid_input("Trash source path is invalid"));
    }
    let parent = source
        .parent()
        .ok_or_else(|| invalid_input("Trash source has no parent"))?;
    let canonical_parent = fs::canonicalize(parent)?;
    let canonical = canonical_parent.join(
        source
            .file_name()
            .ok_or_else(|| invalid_input("Trash source has no file name"))?,
    );
    fs::symlink_metadata(&canonical)?;
    Ok(canonical)
}

fn path_is_normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

fn trash_root_is_structurally_valid(root: &Path) -> bool {
    let uid = effective_uid().to_string();
    let Some(name) = root.file_name() else {
        return false;
    };
    if name == OsStr::new("Trash") || name == OsStr::new(&format!(".Trash-{uid}")) {
        return true;
    }
    name == OsStr::new(&uid)
        && root
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == OsStr::new(".Trash"))
}

#[cfg(any(target_os = "linux", test))]
fn resolve_layout(source: &Path) -> io::Result<TrashLayout> {
    validate_source_path(source)?;
    let mount_points = linux_mount_points()?;
    let source_topdir = containing_mount(source, &mount_points)?;
    if source == source_topdir {
        return Err(invalid_input("a mounted filesystem root cannot be trashed"));
    }

    let home_trash = home_trash_root()?;
    let canonical_home_trash = canonicalize_path_or_parents(&home_trash)?;
    let home_topdir = containing_mount(&canonical_home_trash, &mount_points)?;
    if source_topdir == home_topdir {
        return Ok(TrashLayout {
            topdir: source_topdir.to_path_buf(),
            root: canonical_home_trash,
            path_relative_to_topdir: false,
        });
    }

    let uid = effective_uid();
    let shared = source_topdir.join(".Trash");
    let root = match fs::symlink_metadata(&shared) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.mode() & 0o1000 != 0 =>
        {
            shared.join(uid.to_string())
        }
        Ok(_) | Err(_) => source_topdir.join(format!(".Trash-{uid}")),
    };
    Ok(TrashLayout {
        topdir: source_topdir.to_path_buf(),
        root,
        path_relative_to_topdir: true,
    })
}

#[cfg(any(target_os = "linux", test))]
fn home_trash_root() -> io::Result<PathBuf> {
    let root = match std::env::var_os("XDG_DATA_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path).join("Trash"),
        _ => {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "HOME is unavailable for Trash")
            })?;
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("Trash")
        }
    };
    if !root.is_absolute() {
        return Err(invalid_input("freedesktop Trash path must be absolute"));
    }
    Ok(root)
}

#[cfg(any(target_os = "linux", test))]
fn canonicalize_path_or_parents(path: &Path) -> io::Result<PathBuf> {
    let mut current = path;
    let mut suffix = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(current) {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                suffix.push(
                    current
                        .file_name()
                        .ok_or_else(|| invalid_input("path has no existing ancestor"))?
                        .to_os_string(),
                );
                current = current
                    .parent()
                    .ok_or_else(|| invalid_input("path has no existing ancestor"))?;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_mount_points() -> io::Result<Vec<PathBuf>> {
    const MAX_MOUNTINFO_BYTES: u64 = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    File::open("/proc/self/mountinfo")?
        .take(MAX_MOUNTINFO_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MOUNTINFO_BYTES {
        return Err(invalid_data("mount table exceeds the safety limit"));
    }
    let mut points = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let mount = line
            .split(|byte| *byte == b' ')
            .nth(4)
            .ok_or_else(|| invalid_data("mount table entry is incomplete"))?;
        let decoded = decode_mount_field(mount)?;
        let path = PathBuf::from(OsString::from_vec(decoded));
        if !path.is_absolute() {
            return Err(invalid_data("mount table path is not absolute"));
        }
        points.push(path);
        if points.len() > 4_096 {
            return Err(invalid_data("mount table contains too many entries"));
        }
    }
    points.sort_by(|left, right| {
        right
            .as_os_str()
            .as_bytes()
            .len()
            .cmp(&left.as_os_str().as_bytes().len())
            .then_with(|| left.cmp(right))
    });
    points.dedup();
    if !points.iter().any(|path| path == Path::new("/")) {
        return Err(invalid_data("mount table does not contain the root mount"));
    }
    Ok(points)
}

#[cfg(any(target_os = "linux", test))]
fn decode_mount_field(field: &[u8]) -> io::Result<Vec<u8>> {
    let mut decoded = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] == b'\\' {
            if index + 3 >= field.len()
                || !field[index + 1..=index + 3]
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'7'))
            {
                return Err(invalid_data("mount table escape is invalid"));
            }
            let value = (field[index + 1] - b'0') * 64
                + (field[index + 2] - b'0') * 8
                + (field[index + 3] - b'0');
            decoded.push(value);
            index += 4;
        } else {
            decoded.push(field[index]);
            index += 1;
        }
    }
    Ok(decoded)
}

#[cfg(any(target_os = "linux", test))]
fn containing_mount<'a>(path: &Path, mount_points: &'a [PathBuf]) -> io::Result<&'a Path> {
    mount_points
        .iter()
        .find(|mount| path.starts_with(mount))
        .map(PathBuf::as_path)
        .ok_or_else(|| invalid_data("path is outside the mounted filesystem table"))
}

#[cfg(any(target_os = "linux", test))]
fn existing_trash_layouts() -> io::Result<Vec<TrashLayout>> {
    let mount_points = linux_mount_points()?;
    let home_root = canonicalize_path_or_parents(&home_trash_root()?)?;
    let home_topdir = containing_mount(&home_root, &mount_points)?.to_path_buf();
    let mut layouts = Vec::new();
    if entry_exists(&home_root)? {
        layouts.push(TrashLayout {
            topdir: home_topdir.clone(),
            root: home_root,
            path_relative_to_topdir: false,
        });
    }

    let uid = effective_uid().to_string();
    for topdir in mount_points {
        if topdir == home_topdir {
            continue;
        }
        let shared = topdir.join(".Trash");
        if let Ok(metadata) = fs::symlink_metadata(&shared) {
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.mode() & 0o1000 != 0
            {
                let root = shared.join(&uid);
                match entry_exists(&root) {
                    Ok(true) => layouts.push(TrashLayout {
                        topdir: topdir.clone(),
                        root,
                        path_relative_to_topdir: true,
                    }),
                    Ok(false) => {}
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let root = topdir.join(format!(".Trash-{uid}"));
        match entry_exists(&root) {
            Ok(true) => layouts.push(TrashLayout {
                topdir,
                root,
                path_relative_to_topdir: true,
            }),
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
            Err(error) => return Err(error),
        }
    }
    layouts.sort_by(|left, right| left.root.cmp(&right.root));
    layouts.dedup_by(|left, right| left.root == right.root);
    Ok(layouts)
}

fn validate_existing_layout(layout: &TrashLayout) -> io::Result<()> {
    if !path_is_normal_absolute(&layout.topdir)
        || !path_is_normal_absolute(&layout.root)
        || !layout.root.starts_with(&layout.topdir)
        || !trash_root_is_structurally_valid(&layout.root)
    {
        return Err(invalid_data("existing Trash layout is invalid"));
    }
    let files = layout.root.join("files");
    let info = layout.root.join("info");
    for path in [layout.root.as_path(), files.as_path(), info.as_path()] {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o077 != 0
        {
            return Err(invalid_data(
                "Trash layout is not a private user-owned directory",
            ));
        }
    }
    Ok(())
}

fn list_in_layout(layout: &TrashLayout) -> io::Result<Vec<TrashedItem>> {
    validate_existing_layout(layout)?;
    let info_directory = layout.root.join("info");
    let mut info_paths = Vec::new();
    for entry in fs::read_dir(&info_directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.as_bytes().ends_with(TRASHINFO_SUFFIX.as_bytes()) {
            return Err(invalid_data("unexpected entry in Trash metadata"));
        }
        info_paths.push(entry.path());
        if info_paths.len() > MAX_TRASH_ITEMS {
            return Err(invalid_data("Trash contains too many items"));
        }
    }
    info_paths.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });

    let mut path_bytes = 0usize;
    let mut items = Vec::with_capacity(info_paths.len());
    for info_path in info_paths {
        let file_name = info_path
            .file_name()
            .ok_or_else(|| invalid_data("Trash metadata name is missing"))?;
        let name_bytes = file_name.as_bytes();
        let data_name = &name_bytes[..name_bytes.len() - TRASHINFO_SUFFIX.len()];
        if data_name.is_empty() {
            return Err(invalid_data("Trash data name is empty"));
        }
        let name = OsString::from_vec(data_name.to_vec());
        let data_path = layout.root.join("files").join(&name);
        let (original_path, deleted_at, info_identity, info_sha256) =
            parse_trash_info(layout, &info_path)?;
        path_bytes = path_bytes
            .checked_add(original_path.as_os_str().as_bytes().len())
            .and_then(|bytes| bytes.checked_add(data_path.as_os_str().as_bytes().len()))
            .filter(|bytes| *bytes <= MAX_TRASH_PATH_BYTES)
            .ok_or_else(|| invalid_data("Trash paths exceed the safety limit"))?;
        let data_identity = EntryIdentity::capture(&data_path)?;
        let data_manifest = TreeManifest::capture(&data_path)?;
        if EntryIdentity::capture(&data_path)? != data_identity {
            return Err(changed("Trash data changed while it was listed"));
        }
        items.push(TrashedItem {
            name,
            original_path,
            deleted_at,
            layout: layout.clone(),
            data_path,
            info_path,
            data_identity,
            data_manifest,
            info_identity,
            info_sha256,
        });
    }
    items.sort_by(|left, right| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| left.name.as_bytes().cmp(right.name.as_bytes()))
    });
    Ok(items)
}

fn parse_trash_info(
    layout: &TrashLayout,
    info_path: &Path,
) -> io::Result<(PathBuf, String, EntryIdentity, [u8; 32])> {
    let metadata = fs::symlink_metadata(info_path)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o022 != 0
        || metadata.len() > MAX_INFO_BYTES
    {
        return Err(invalid_data("Trash metadata is not a safe regular file"));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(info_path)?;
    let before = EntryIdentity::capture_file(&file)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INFO_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INFO_BYTES || EntryIdentity::capture(info_path)? != before {
        return Err(changed("Trash metadata changed while it was read"));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| invalid_data("Trash metadata is not valid UTF-8"))?;
    let mut lines = text.lines();
    if lines.next() != Some("[Trash Info]") {
        return Err(invalid_data("Trash metadata header is invalid"));
    }
    let mut encoded_path = None;
    let mut deleted_at = None;
    for line in lines {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid_data("Trash metadata entry is invalid"));
        };
        match key {
            "Path" => {
                if encoded_path.replace(value).is_some() {
                    return Err(invalid_data("Trash metadata has duplicate Path"));
                }
            }
            "DeletionDate" => {
                if deleted_at.replace(value).is_some() {
                    return Err(invalid_data("Trash metadata has duplicate DeletionDate"));
                }
            }
            _ => {}
        }
    }
    let encoded_path = encoded_path.ok_or_else(|| invalid_data("Trash metadata has no Path"))?;
    let deleted_at =
        deleted_at.ok_or_else(|| invalid_data("Trash metadata has no DeletionDate"))?;
    NaiveDateTime::parse_from_str(deleted_at, "%Y-%m-%dT%H:%M:%S")
        .map_err(|_| invalid_data("Trash deletion date is invalid"))?;
    let decoded = percent_decode_path(encoded_path.as_bytes())?;
    let decoded = PathBuf::from(OsString::from_vec(decoded));
    let original_path = if layout.path_relative_to_topdir {
        if decoded.is_absolute()
            || decoded.as_os_str().is_empty()
            || !relative_path_is_normal(&decoded)
        {
            return Err(invalid_data("mounted Trash identity is invalid"));
        }
        layout.topdir.join(decoded)
    } else {
        if !path_is_normal_absolute(&decoded) {
            return Err(invalid_data("home Trash identity is invalid"));
        }
        decoded
    };
    Ok((
        original_path,
        deleted_at.to_string(),
        before,
        sha256(&bytes),
    ))
}

fn percent_decode_path(encoded: &[u8]) -> io::Result<Vec<u8>> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] != b'%' {
            if encoded[index] == 0 {
                return Err(invalid_data("Trash identity contains NUL"));
            }
            decoded.push(encoded[index]);
            index += 1;
            continue;
        }
        if index + 2 >= encoded.len() {
            return Err(invalid_data("Trash identity escape is incomplete"));
        }
        let high = hex_value(encoded[index + 1])
            .ok_or_else(|| invalid_data("Trash identity escape is invalid"))?;
        let low = hex_value(encoded[index + 2])
            .ok_or_else(|| invalid_data("Trash identity escape is invalid"))?;
        let byte = high * 16 + low;
        if byte == 0 {
            return Err(invalid_data("Trash identity contains NUL"));
        }
        decoded.push(byte);
        index += 3;
    }
    Ok(decoded)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn relative_path_is_normal(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn suffixed_name(original: &OsStr, suffix: usize) -> OsString {
    let suffix = if suffix == 1 {
        String::new()
    } else {
        format!(".{suffix}")
    };
    let available = MAX_FILENAME_BYTES
        .saturating_sub(TRASHINFO_SUFFIX.len())
        .saturating_sub(suffix.len());
    let mut bytes = original.as_bytes()[..original.as_bytes().len().min(available)].to_vec();
    bytes.extend_from_slice(suffix.as_bytes());
    OsString::from_vec(bytes)
}

fn recovered_name(original: &OsStr, id: &str, suffix: usize) -> OsString {
    let short_id = id.get(..8).unwrap_or(id);
    let suffix = if suffix == 1 {
        format!(" (Recovered {short_id})")
    } else {
        format!(" (Recovered {short_id}-{suffix})")
    };
    let available = MAX_FILENAME_BYTES
        .saturating_sub(TRASHINFO_SUFFIX.len())
        .saturating_sub(suffix.len());
    let mut bytes = original.as_bytes()[..original.as_bytes().len().min(available)].to_vec();
    bytes.extend_from_slice(suffix.as_bytes());
    OsString::from_vec(bytes)
}

fn trash_info_bytes(
    source: &Path,
    layout: &TrashLayout,
    deleted_at: DateTime<Local>,
) -> io::Result<Vec<u8>> {
    let path = if layout.path_relative_to_topdir {
        source
            .strip_prefix(&layout.topdir)
            .map_err(|_| invalid_input("Trash source is outside its mounted filesystem"))?
    } else {
        source
    };
    if path.as_os_str().is_empty() {
        return Err(invalid_input("Trash source identity is empty"));
    }
    let encoded = percent_encode_path(path.as_os_str().as_bytes());
    let bytes = format!(
        "[Trash Info]\nPath={encoded}\nDeletionDate={}\n",
        deleted_at.format("%Y-%m-%dT%H:%M:%S")
    )
    .into_bytes();
    if bytes.len() as u64 > MAX_INFO_BYTES {
        return Err(invalid_input("Trash metadata exceeds the safety limit"));
    }
    Ok(bytes)
}

fn recovery_layout(record: &TrashRecord) -> io::Result<TrashLayout> {
    let root = record.trash_root();
    let uid = effective_uid().to_string();
    let name = root
        .file_name()
        .ok_or_else(|| invalid_data("Trash recovery root has no name"))?;
    if name == OsStr::new("Trash") {
        return Ok(TrashLayout {
            topdir: PathBuf::from("/"),
            root,
            path_relative_to_topdir: false,
        });
    }
    if name == OsStr::new(&format!(".Trash-{uid}")) {
        let topdir = root
            .parent()
            .ok_or_else(|| invalid_data("mounted Trash root has no top directory"))?
            .to_path_buf();
        return Ok(TrashLayout {
            topdir,
            root,
            path_relative_to_topdir: true,
        });
    }
    if name == OsStr::new(&uid)
        && root
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == OsStr::new(".Trash"))
    {
        let topdir = root
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| invalid_data("shared Trash root has no top directory"))?
            .to_path_buf();
        return Ok(TrashLayout {
            topdir,
            root,
            path_relative_to_topdir: true,
        });
    }
    Err(invalid_data(
        "Trash recovery root cannot rebuild metadata safely",
    ))
}

fn percent_encode_path(path: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(path.len());
    for byte in path {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn create_info_file(path: &Path, bytes: &[u8]) -> io::Result<EntryIdentity> {
    if bytes.len() as u64 > MAX_INFO_BYTES {
        return Err(invalid_input("Trash metadata exceeds the safety limit"));
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    let identity = EntryIdentity::capture_file(&file)?;
    drop(file);
    sync_directory(
        path.parent()
            .ok_or_else(|| invalid_data("Trash info parent is missing"))?,
    )?;
    Ok(identity)
}

fn record_source_matches(record: &TrashRecord) -> io::Result<bool> {
    record_source_matches_with_cancel(record, None)
}

fn record_source_matches_with_cancel(
    record: &TrashRecord,
    cancel: Option<&AtomicBool>,
) -> io::Result<bool> {
    let identity = match EntryIdentity::capture(&record.source()) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if identity != record.source_identity {
        return Ok(false);
    }
    let manifest = match cancel {
        Some(cancel) => TreeManifest::capture_cancellable(&record.source(), cancel),
        None => TreeManifest::capture(&record.source()),
    };
    match manifest {
        Ok(manifest) => Ok(manifest == record.source_manifest),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn record_data_matches_after_rename(record: &TrashRecord) -> io::Result<bool> {
    record_tree_matches(record, &record.data_path(), true)
}

fn record_data_matches_exact(record: &TrashRecord) -> io::Result<bool> {
    record_tree_matches(record, &record.data_path(), false)
}

fn record_restored_data_matches(record: &TrashRecord) -> io::Result<bool> {
    record_tree_matches(record, &record.source(), true)
}

fn record_delete_data_matches_exact(record: &TrashRecord) -> io::Result<bool> {
    let delete = record
        .delete_path()
        .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
    record_tree_matches(record, &delete, true)
}

fn record_delete_directory_root_matches(record: &TrashRecord) -> io::Result<bool> {
    let delete = record
        .delete_path()
        .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
    let metadata = match fs::symlink_metadata(&delete) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let current = EntryIdentity::capture(&delete)?;
    Ok(record.source_identity.same_object(&current))
}

fn record_tree_matches(record: &TrashRecord, path: &Path, published: bool) -> io::Result<bool> {
    let identity = match EntryIdentity::capture(path) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let identity_matches = if published {
        record.source_identity.same_entry_after_rename(&identity)
    } else {
        record.source_identity == identity
    };
    if !identity_matches {
        return Ok(false);
    }
    match TreeManifest::capture(path) {
        Ok(manifest) => Ok(if published {
            record.source_manifest.same_after_root_rename(&manifest)
        } else {
            record.source_manifest == manifest
        }),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn resolution_tree_matches(record: &TrashRecord, path: &Path, published: bool) -> io::Result<bool> {
    let (Some(expected_identity), Some(expected_manifest)) = (
        record.resolution_identity.as_ref(),
        record.resolution_manifest.as_ref(),
    ) else {
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

fn resolution_primary_tree_matches(record: &TrashRecord) -> io::Result<bool> {
    let (Some(expected_identity), Some(expected_manifest)) = (
        record.resolution_primary_identity.as_ref(),
        record.resolution_primary_manifest.as_ref(),
    ) else {
        return Ok(false);
    };
    let path = record.data_path();
    let current_identity = match EntryIdentity::capture(&path) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if expected_identity != &current_identity {
        return Ok(false);
    }
    let current_manifest = match TreeManifest::capture(&path) {
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
    Ok(expected_manifest == &current_manifest)
}

fn resolution_info_matches(record: &TrashRecord) -> io::Result<bool> {
    let (Some(expected_identity), Some(expected_sha256)) = (
        record.resolution_info_identity.as_ref(),
        record.resolution_info_sha256,
    ) else {
        return Ok(false);
    };
    let current = match capture_optional_info(&record.info_path()) {
        Ok(snapshot) => snapshot,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(current.is_some_and(|snapshot| {
        &snapshot.identity == expected_identity && snapshot.sha256 == expected_sha256
    }))
}

fn recovered_resolution_info_matches(record: &TrashRecord) -> io::Result<bool> {
    let (Some(path), Some(expected_identity), Some(expected_sha256)) = (
        record.resolution_info_path(),
        record.resolution_info_identity.as_ref(),
        record.resolution_info_sha256,
    ) else {
        return Ok(false);
    };
    let current = match capture_optional_info(&path) {
        Ok(snapshot) => snapshot,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(current.is_some_and(|snapshot| {
        &snapshot.identity == expected_identity && snapshot.sha256 == expected_sha256
    }))
}

fn restore_parent_matches(record: &TrashRecord, exact: bool) -> io::Result<bool> {
    let Some(expected) = &record.restore_parent_identity else {
        return Ok(false);
    };
    let parent = record
        .source()
        .parent()
        .ok_or_else(|| invalid_data("restore destination has no parent"))?
        .to_path_buf();
    let current = match EntryIdentity::capture(&parent) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    Ok(if exact {
        expected == &current
    } else {
        expected.same_object(&current)
    })
}

fn record_info_matches(record: &TrashRecord) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(record.info_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o022 != 0
        || metadata.uid() != effective_uid()
        || metadata.len() > MAX_INFO_BYTES
    {
        return Ok(false);
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(record.info_path())?;
    let identity = EntryIdentity::capture_file(&file)?;
    if let Some(expected) = &record.info_identity {
        if &identity != expected {
            return Ok(false);
        }
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INFO_BYTES + 1).read_to_end(&mut bytes)?;
    Ok(bytes.len() as u64 <= MAX_INFO_BYTES && sha256(&bytes) == record.info_sha256)
}

fn read_bound_info_bytes(record: &TrashRecord) -> io::Result<Option<Vec<u8>>> {
    let path = record.info_path();
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o022 != 0
        || metadata.uid() != effective_uid()
        || metadata.len() > MAX_INFO_BYTES
    {
        return Err(changed(
            "Trash metadata is not a bounded private regular file",
        ));
    }
    let expected = record
        .info_identity
        .as_ref()
        .ok_or_else(|| invalid_data("Trash transaction has no metadata identity"))?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(&path)?;
    let identity = EntryIdentity::capture_file(&file)?;
    if &identity != expected || EntryIdentity::capture(&path)? != identity {
        return Err(changed("Trash metadata changed before Undo was archived"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(MAX_INFO_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INFO_BYTES
        || sha256(&bytes) != record.info_sha256
        || EntryIdentity::capture_file(&file)? != identity
        || EntryIdentity::capture(&path)? != identity
    {
        return Err(changed("Trash metadata changed before Undo was archived"));
    }
    Ok(Some(bytes))
}

fn remove_exact_info(record: &TrashRecord) -> io::Result<()> {
    if !record_info_matches(record)? {
        return Err(changed("Trash metadata changed before cleanup"));
    }
    fs::remove_file(record.info_path())?;
    sync_directory(
        record
            .info_path()
            .parent()
            .ok_or_else(|| invalid_data("Trash info parent is missing"))?,
    )
}

fn remove_staged_delete_data(record: &TrashRecord) -> io::Result<()> {
    let delete = record
        .delete_path()
        .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
    let metadata = fs::symlink_metadata(&delete)?;
    let identity = EntryIdentity::capture(&delete)?;
    if !record.source_identity.same_object(&identity) {
        return Err(changed("permanent-delete staging data changed"));
    }
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(&delete)?;
    } else {
        if !record_delete_data_matches_exact(record)? {
            return Err(changed("permanent-delete staging data changed"));
        }
        fs::remove_file(&delete)?;
    }
    sync_directory(
        delete
            .parent()
            .ok_or_else(|| invalid_data("permanent-delete staging parent is missing"))?,
    )
}

fn entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn capture_optional_tree(path: &Path) -> io::Result<Option<TreeSnapshot>> {
    let identity = match EntryIdentity::capture(path) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let manifest = TreeManifest::capture(path)?;
    if EntryIdentity::capture(path)? != identity {
        return Err(changed("Trash recovery item changed while it was reviewed"));
    }
    Ok(Some(TreeSnapshot { identity, manifest }))
}

fn capture_optional_info(path: &Path) -> io::Result<Option<InfoSnapshot>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o022 != 0
        || metadata.uid() != effective_uid()
        || metadata.len() > MAX_INFO_BYTES
    {
        return Err(changed(
            "Trash recovery metadata is not a bounded private regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path)?;
    let identity = EntryIdentity::capture_file(&file)?;
    if EntryIdentity::capture(path)? != identity {
        return Err(changed(
            "Trash recovery metadata changed while it was reviewed",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(MAX_INFO_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INFO_BYTES
        || EntryIdentity::capture_file(&file)? != identity
        || EntryIdentity::capture(path)? != identity
    {
        return Err(changed(
            "Trash recovery metadata changed while it was reviewed",
        ));
    }
    Ok(Some(InfoSnapshot {
        identity,
        sha256: sha256(&bytes),
    }))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest.finalize().into()
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
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

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn changed(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, message)
}

fn review_changed() -> io::Error {
    changed("Trash recovery changed; review it again")
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Move to Trash cancelled")
}

fn interrupted_restore() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Restore cancelled")
}

fn interrupted_delete() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Permanent deletion cancelled before it began",
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrashLayout {
    topdir: PathBuf,
    root: PathBuf,
    path_relative_to_topdir: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TrashedItem {
    pub(crate) name: OsString,
    pub(crate) original_path: PathBuf,
    pub(crate) deleted_at: String,
    layout: TrashLayout,
    data_path: PathBuf,
    info_path: PathBuf,
    data_identity: EntryIdentity,
    data_manifest: TreeManifest,
    info_identity: EntryIdentity,
    info_sha256: [u8; 32],
}

impl fmt::Debug for TrashedItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrashedItem")
            .field("deleted_at", &self.deleted_at)
            .finish_non_exhaustive()
    }
}

impl TrashedItem {
    pub(crate) fn data_path(&self) -> &Path {
        &self.data_path
    }
}

#[derive(Clone, PartialEq, Eq)]
struct TreeSnapshot {
    identity: EntryIdentity,
    manifest: TreeManifest,
}

#[derive(Clone, PartialEq, Eq)]
struct InfoSnapshot {
    identity: EntryIdentity,
    sha256: [u8; 32],
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum TrashRecoveryAction {
    ReturnRemainingItem { may_be_partial: bool },
    ReturnRemainingItemAndRebuildMetadata { may_be_partial: bool },
    PreserveConflictingItems { rebuild_metadata: bool },
    RebuildMetadata,
    RemoveOrphanMetadata,
    KeepExistingItems,
    RequiresManualRepair,
}

impl fmt::Debug for TrashRecoveryAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReturnRemainingItem { may_be_partial } => formatter
                .debug_struct("ReturnRemainingItem")
                .field("may_be_partial", may_be_partial)
                .finish(),
            Self::ReturnRemainingItemAndRebuildMetadata { may_be_partial } => formatter
                .debug_struct("ReturnRemainingItemAndRebuildMetadata")
                .field("may_be_partial", may_be_partial)
                .finish(),
            Self::PreserveConflictingItems { rebuild_metadata } => formatter
                .debug_struct("PreserveConflictingItems")
                .field("rebuild_metadata", rebuild_metadata)
                .finish(),
            Self::RebuildMetadata => formatter.write_str("RebuildMetadata"),
            Self::RemoveOrphanMetadata => formatter.write_str("RemoveOrphanMetadata"),
            Self::KeepExistingItems => formatter.write_str("KeepExistingItems"),
            Self::RequiresManualRepair => formatter.write_str("RequiresManualRepair"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct TrashRecoveryReview {
    record: TrashRecord,
    journal_identity: EntryIdentity,
    source_snapshot: Option<TreeSnapshot>,
    data_snapshot: Option<TreeSnapshot>,
    delete_snapshot: Option<TreeSnapshot>,
    info_snapshot: Option<InfoSnapshot>,
    pub(crate) action: TrashRecoveryAction,
}

impl fmt::Debug for TrashRecoveryReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrashRecoveryReview")
            .field("operation", &self.record.operation)
            .field("stage", &self.record.stage)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrashResolutionOutcome {
    ReturnedRemainingItem { may_be_partial: bool },
    ReturnedRemainingItemAndRebuiltMetadata { may_be_partial: bool },
    PreservedConflictingItems { rebuilt_metadata: bool },
    RebuiltMetadata,
    RemovedOrphanMetadata,
    KeptExistingItems,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TrashRecovery {
    pub(crate) finalized: usize,
    pub(crate) pending: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TrashStore {
    state_root: PathBuf,
    undo: UndoStore,
}

impl TrashStore {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn open_default() -> io::Result<Self> {
        let state_root = default_state_root()?.join("trash-operations");
        Self::open(state_root)
    }

    pub(crate) fn open(state_root: PathBuf) -> io::Result<Self> {
        ensure_private_directory(&state_root)?;
        let operations_root = state_root
            .parent()
            .ok_or_else(|| invalid_input("Trash state root has no parent"))?
            .join("operations");
        ensure_private_directory(&operations_root)?;
        let undo = UndoStore::open(operations_root.join("undo"))?;
        Ok(Self { state_root, undo })
    }

    pub(crate) fn undo_store(&self) -> &UndoStore {
        &self.undo
    }

    pub(crate) fn execute_latest_undo(
        &self,
        fs: &impl crate::file_ops::FileSystem,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(crate::file_ops::CopyActivity),
    ) -> io::Result<Option<UndoOutcome>> {
        let _lock = self.lock()?;
        let recovery = self.recover_locked()?;
        if recovery.pending != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "unfinished Trash recovery must be reviewed before Undo",
            ));
        }
        self.undo.execute_latest(fs, cancel, progress)
    }

    #[cfg(test)]
    pub(crate) fn recover(&self) -> io::Result<TrashRecovery> {
        let _lock = self.lock()?;
        self.recover_locked()
    }

    pub(crate) fn recover_and_review(
        &self,
    ) -> io::Result<(TrashRecovery, Vec<TrashRecoveryReview>)> {
        let _lock = self.lock()?;
        let recovery = self.recover_locked()?;
        let reviews = self.review_pending_locked()?;
        Ok((recovery, reviews))
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn list(&self) -> io::Result<Vec<TrashedItem>> {
        let _lock = self.lock()?;
        let recovery = self.recover_locked()?;
        if recovery.pending != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "unfinished Trash recovery must be reviewed first",
            ));
        }
        let mut items = Vec::new();
        for layout in existing_trash_layouts()? {
            items.extend(list_in_layout(&layout)?);
            if items.len() > MAX_TRASH_ITEMS {
                return Err(invalid_data("Trash contains too many items"));
            }
        }
        items.sort_by(|left, right| {
            right
                .deleted_at
                .cmp(&left.deleted_at)
                .then_with(|| left.name.as_bytes().cmp(right.name.as_bytes()))
        });
        Ok(items)
    }

    #[cfg(test)]
    pub(crate) fn review_pending(&self) -> io::Result<Vec<TrashRecoveryReview>> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        self.review_pending_locked()
    }

    fn review_pending_locked(&self) -> io::Result<Vec<TrashRecoveryReview>> {
        let mut reviews = Vec::new();
        for (path, record) in self.read_records()? {
            let journal_identity = EntryIdentity::capture(&path)?;
            let source_snapshot = capture_optional_tree(&record.source())?;
            let data_snapshot = capture_optional_tree(&record.data_path())?;
            let delete_snapshot = match record.delete_path() {
                Some(path) => capture_optional_tree(&path)?,
                None => None,
            };
            let info_snapshot = capture_optional_info(&record.info_path())?;
            let action = if record.operation == TrashOperation::Delete
                && record.stage == TrashStage::DeleteDataStaged
                && data_snapshot.is_none()
                && delete_snapshot.is_some()
                && info_snapshot.is_some()
            {
                let snapshot = delete_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("delete review has no staged item"))?;
                let original_matches = record
                    .source_identity
                    .same_entry_after_rename(&snapshot.identity)
                    && record
                        .source_manifest
                        .same_after_root_rename(&snapshot.manifest);
                TrashRecoveryAction::ReturnRemainingItem {
                    may_be_partial: !original_matches,
                }
            } else if record.operation == TrashOperation::Delete
                && record.stage == TrashStage::DeleteDataStaged
                && data_snapshot.is_none()
                && delete_snapshot.is_some()
                && info_snapshot.is_none()
            {
                let snapshot = delete_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("delete review has no staged item"))?;
                let original_matches = record
                    .source_identity
                    .same_entry_after_rename(&snapshot.identity)
                    && record
                        .source_manifest
                        .same_after_root_rename(&snapshot.manifest);
                TrashRecoveryAction::ReturnRemainingItemAndRebuildMetadata {
                    may_be_partial: !original_matches,
                }
            } else if data_snapshot.is_some()
                && delete_snapshot.is_none()
                && info_snapshot.is_none()
            {
                TrashRecoveryAction::RebuildMetadata
            } else if record.operation == TrashOperation::Delete
                && record.stage == TrashStage::DeleteDataStaged
                && data_snapshot.is_some()
                && delete_snapshot.is_some()
                && info_snapshot.as_ref().is_none_or(|snapshot| {
                    record.info_identity.as_ref() == Some(&snapshot.identity)
                        && record.info_sha256 == snapshot.sha256
                })
            {
                TrashRecoveryAction::PreserveConflictingItems {
                    rebuild_metadata: info_snapshot.is_none(),
                }
            } else if data_snapshot.is_none()
                && delete_snapshot.is_none()
                && info_snapshot.is_some()
            {
                TrashRecoveryAction::RemoveOrphanMetadata
            } else if delete_snapshot.is_some()
                || (data_snapshot.is_some() && info_snapshot.is_none())
            {
                TrashRecoveryAction::RequiresManualRepair
            } else {
                TrashRecoveryAction::KeepExistingItems
            };
            reviews.push(TrashRecoveryReview {
                record,
                journal_identity,
                source_snapshot,
                data_snapshot,
                delete_snapshot,
                info_snapshot,
                action,
            });
        }
        Ok(reviews)
    }

    #[cfg(test)]
    pub(crate) fn resolve_review(
        &self,
        review: &TrashRecoveryReview,
    ) -> io::Result<TrashResolutionOutcome> {
        let _lock = self.lock()?;
        self.resolve_review_locked(review)
    }

    pub(crate) fn resolve_review_and_refresh(
        &self,
        review: &TrashRecoveryReview,
    ) -> io::Result<(
        TrashResolutionOutcome,
        TrashRecovery,
        Vec<TrashRecoveryReview>,
        Option<UndoAvailability>,
    )> {
        let _lock = self.lock()?;
        let outcome = self.resolve_review_locked(review)?;
        let recovery = self.recover_locked()?;
        let reviews = self.review_pending_locked()?;
        let undo = self.undo.latest()?;
        Ok((outcome, recovery, reviews, undo))
    }

    fn resolve_review_locked(
        &self,
        review: &TrashRecoveryReview,
    ) -> io::Result<TrashResolutionOutcome> {
        self.validate_review(review)?;
        let record_path = self.record_path(&review.record.id);
        match &review.action {
            TrashRecoveryAction::ReturnRemainingItem { may_be_partial } => {
                let delete_snapshot = review
                    .delete_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("delete recovery has no staged item"))?;
                let info_snapshot = review
                    .info_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("delete recovery has no metadata"))?;
                if review.data_snapshot.is_some() {
                    return Err(review_changed());
                }
                let mut record = review.record.clone();
                record.stage = TrashStage::DeleteDataStaged;
                record.resolution_intent = Some(TrashResolutionIntent::ReturnDeleteStage);
                record.resolution_identity = Some(delete_snapshot.identity.clone());
                record.resolution_manifest = Some(delete_snapshot.manifest.clone());
                record.resolution_info_identity = Some(info_snapshot.identity.clone());
                record.resolution_info_sha256 = Some(info_snapshot.sha256);
                record.validate(&record.id)?;
                self.persist(&record_path, &record, false)?;
                if !self.recover_returned_delete_stage(&record_path, &mut record)? {
                    return Err(review_changed());
                }
                Ok(TrashResolutionOutcome::ReturnedRemainingItem {
                    may_be_partial: *may_be_partial,
                })
            }
            TrashRecoveryAction::ReturnRemainingItemAndRebuildMetadata { may_be_partial } => {
                let delete_snapshot = review
                    .delete_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("delete recovery has no staged item"))?;
                if review.data_snapshot.is_some() || review.info_snapshot.is_some() {
                    return Err(review_changed());
                }
                let mut record = review.record.clone();
                self.begin_metadata_rebuild(
                    &record_path,
                    &mut record,
                    TrashResolutionIntent::ReturnDeleteStageAndRebuildMetadata,
                    delete_snapshot,
                )?;
                if !self.recover_rebuilt_metadata(&record_path, &mut record)? {
                    return Err(review_changed());
                }
                Ok(
                    TrashResolutionOutcome::ReturnedRemainingItemAndRebuiltMetadata {
                        may_be_partial: *may_be_partial,
                    },
                )
            }
            TrashRecoveryAction::PreserveConflictingItems { rebuild_metadata } => {
                let primary = review
                    .data_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("conflict recovery has no visible item"))?;
                let staged = review
                    .delete_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("conflict recovery has no staged item"))?;
                if review.info_snapshot.is_none() != *rebuild_metadata {
                    return Err(review_changed());
                }
                let mut record = review.record.clone();
                self.begin_conflicting_items_recovery(
                    &record_path,
                    &mut record,
                    primary,
                    staged,
                    review.info_snapshot.as_ref(),
                )?;
                if !self.recover_conflicting_items(&record_path, &mut record)? {
                    return Err(review_changed());
                }
                Ok(TrashResolutionOutcome::PreservedConflictingItems {
                    rebuilt_metadata: *rebuild_metadata,
                })
            }
            TrashRecoveryAction::RebuildMetadata => {
                let data_snapshot = review
                    .data_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("metadata recovery has no Trash item"))?;
                if review.delete_snapshot.is_some() || review.info_snapshot.is_some() {
                    return Err(review_changed());
                }
                let mut record = review.record.clone();
                self.begin_metadata_rebuild(
                    &record_path,
                    &mut record,
                    TrashResolutionIntent::RebuildMetadata,
                    data_snapshot,
                )?;
                if !self.recover_rebuilt_metadata(&record_path, &mut record)? {
                    return Err(review_changed());
                }
                Ok(TrashResolutionOutcome::RebuiltMetadata)
            }
            TrashRecoveryAction::RemoveOrphanMetadata => {
                let info_snapshot = review
                    .info_snapshot
                    .as_ref()
                    .ok_or_else(|| invalid_data("orphan recovery has no metadata"))?;
                if review.data_snapshot.is_some() || review.delete_snapshot.is_some() {
                    return Err(review_changed());
                }
                if capture_optional_info(&review.record.info_path())?.as_ref()
                    != Some(info_snapshot)
                {
                    return Err(review_changed());
                }
                fs::remove_file(review.record.info_path())?;
                sync_directory(
                    review
                        .record
                        .info_path()
                        .parent()
                        .ok_or_else(|| invalid_data("Trash info parent is missing"))?,
                )?;
                self.finish_record(&record_path)?;
                Ok(TrashResolutionOutcome::RemovedOrphanMetadata)
            }
            TrashRecoveryAction::KeepExistingItems => {
                self.finish_record(&record_path)?;
                Ok(TrashResolutionOutcome::KeptExistingItems)
            }
            TrashRecoveryAction::RequiresManualRepair => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Trash recovery requires manual repair",
            )),
        }
    }

    fn validate_review(&self, review: &TrashRecoveryReview) -> io::Result<()> {
        let record_path = self.record_path(&review.record.id);
        if EntryIdentity::capture(&record_path)? != review.journal_identity {
            return Err(review_changed());
        }
        let current = self
            .read_records()?
            .into_iter()
            .find_map(|(_, record)| (record.id == review.record.id).then_some(record))
            .ok_or_else(review_changed)?;
        let delete_snapshot = match review.record.delete_path() {
            Some(path) => capture_optional_tree(&path)?,
            None => None,
        };
        if current != review.record
            || capture_optional_tree(&review.record.source())? != review.source_snapshot
            || capture_optional_tree(&review.record.data_path())? != review.data_snapshot
            || delete_snapshot != review.delete_snapshot
            || capture_optional_info(&review.record.info_path())? != review.info_snapshot
        {
            return Err(review_changed());
        }
        Ok(())
    }

    fn begin_metadata_rebuild(
        &self,
        record_path: &Path,
        record: &mut TrashRecord,
        intent: TrashResolutionIntent,
        item: &TreeSnapshot,
    ) -> io::Result<()> {
        let layout = recovery_layout(record)?;
        let info_bytes = trash_info_bytes(&record.source(), &layout, Local::now())?;
        record.resolution_intent = Some(intent);
        record.resolution_identity = Some(item.identity.clone());
        record.resolution_manifest = Some(item.manifest.clone());
        record.resolution_info_identity = None;
        record.resolution_info_sha256 = Some(sha256(&info_bytes));
        record.resolution_info_bytes = Some(info_bytes);
        record.validate(&record.id)?;
        self.persist(record_path, record, false)
    }

    fn begin_conflicting_items_recovery(
        &self,
        record_path: &Path,
        record: &mut TrashRecord,
        primary: &TreeSnapshot,
        staged: &TreeSnapshot,
        primary_info: Option<&InfoSnapshot>,
    ) -> io::Result<()> {
        if record.operation != TrashOperation::Delete
            || record.stage != TrashStage::DeleteDataStaged
        {
            return Err(invalid_data(
                "conflicting-item recovery requires a staged deletion",
            ));
        }
        if primary_info.is_some_and(|snapshot| {
            record.info_identity.as_ref() != Some(&snapshot.identity)
                || record.info_sha256 != snapshot.sha256
        }) {
            return Err(review_changed());
        }

        let layout = recovery_layout(record)?;
        let info_bytes = if primary_info.is_some() {
            read_bound_info_bytes(record)?
                .ok_or_else(|| changed("reviewed Trash metadata disappeared"))?
        } else {
            trash_info_bytes(&record.source(), &layout, Local::now())?
        };
        let original_name = record
            .data_path()
            .file_name()
            .ok_or_else(|| invalid_data("Trash item name is missing"))?
            .to_os_string();
        let (data_path, info_path) = (1..=10_000usize)
            .find_map(|suffix| {
                let name = recovered_name(&original_name, &record.id, suffix);
                let data_path = layout.root.join("files").join(&name);
                let mut info_name = name;
                info_name.push(TRASHINFO_SUFFIX);
                let info_path = layout.root.join("info").join(info_name);
                match (entry_exists(&data_path), entry_exists(&info_path)) {
                    (Ok(false), Ok(false)) => Some(Ok((data_path, info_path))),
                    (Ok(_), Ok(_)) => None,
                    (Err(error), _) | (_, Err(error)) => Some(Err(error)),
                }
            })
            .transpose()?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "no unique recovered Trash name is available",
                )
            })?;

        record.resolution_intent = Some(TrashResolutionIntent::PreserveConflictingItems);
        record.resolution_identity = Some(staged.identity.clone());
        record.resolution_manifest = Some(staged.manifest.clone());
        record.resolution_info_identity = None;
        record.resolution_info_sha256 = Some(sha256(&info_bytes));
        record.resolution_info_bytes = Some(info_bytes);
        record.resolution_primary_identity = Some(primary.identity.clone());
        record.resolution_primary_manifest = Some(primary.manifest.clone());
        record.resolution_primary_info_identity =
            primary_info.map(|snapshot| snapshot.identity.clone());
        record.resolution_data_path_bytes = Some(data_path.as_os_str().as_bytes().to_vec());
        record.resolution_info_path_bytes = Some(info_path.as_os_str().as_bytes().to_vec());
        record.validate(&record.id)?;
        self.persist(record_path, record, false)
    }

    fn recover_conflicting_items(
        &self,
        record_path: &Path,
        record: &mut TrashRecord,
    ) -> io::Result<bool> {
        if record.resolution_intent != Some(TrashResolutionIntent::PreserveConflictingItems)
            || !resolution_primary_tree_matches(record)?
        {
            return Ok(false);
        }
        let expected_info_sha = record
            .resolution_info_sha256
            .ok_or_else(|| invalid_data("conflict recovery has no metadata digest"))?;
        let info_bytes = record
            .resolution_info_bytes
            .as_ref()
            .ok_or_else(|| invalid_data("conflict recovery has no metadata bytes"))?
            .clone();

        let primary_info = capture_optional_info(&record.info_path())?;
        match (
            record.resolution_primary_info_identity.as_ref(),
            primary_info,
        ) {
            (Some(expected), Some(snapshot))
                if expected == &snapshot.identity && record.info_sha256 == snapshot.sha256 => {}
            (Some(_), _) => return Ok(false),
            (None, Some(snapshot)) if snapshot.sha256 == expected_info_sha => {
                record.info_identity = Some(snapshot.identity.clone());
                record.info_sha256 = expected_info_sha;
                record.resolution_primary_info_identity = Some(snapshot.identity);
                self.persist(record_path, record, false)?;
            }
            (None, Some(_)) => return Ok(false),
            (None, None) => {
                let identity = match create_info_file(&record.info_path(), &info_bytes) {
                    Ok(identity) => identity,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                };
                record.info_identity = Some(identity.clone());
                record.info_sha256 = expected_info_sha;
                record.resolution_primary_info_identity = Some(identity);
                self.persist(record_path, record, false)?;
            }
        }
        if !resolution_primary_tree_matches(record)? || !record_info_matches(record)? {
            return Ok(false);
        }

        let delete_path = record
            .delete_path()
            .ok_or_else(|| invalid_data("conflict recovery has no delete stage"))?;
        let recovered_data = record
            .resolution_data_path()
            .ok_or_else(|| invalid_data("conflict recovery has no recovered path"))?;
        let delete_exists = entry_exists(&delete_path)?;
        let recovered_exists = entry_exists(&recovered_data)?;
        if delete_exists && recovered_exists {
            return Ok(false);
        }
        if delete_exists && !resolution_tree_matches(record, &delete_path, false)? {
            return Ok(false);
        }

        let recovered_info = record
            .resolution_info_path()
            .ok_or_else(|| invalid_data("conflict recovery has no recovered metadata path"))?;
        let info_identity = match capture_optional_info(&recovered_info)? {
            Some(snapshot)
                if snapshot.sha256 == expected_info_sha
                    && record
                        .resolution_info_identity
                        .as_ref()
                        .is_none_or(|expected| expected == &snapshot.identity) =>
            {
                snapshot.identity
            }
            Some(_) => return Ok(false),
            None => match create_info_file(&recovered_info, &info_bytes) {
                Ok(identity) => identity,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
                Err(error) => return Err(error),
            },
        };
        record.resolution_info_identity = Some(info_identity);
        self.persist(record_path, record, false)?;
        if delete_exists {
            match rename_noreplace(&delete_path, &recovered_data) {
                Ok(()) => {
                    sync_directory(
                        recovered_data
                            .parent()
                            .ok_or_else(|| invalid_data("Trash data parent is missing"))?,
                    )?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if recovered_resolution_info_matches(record)? {
                        fs::remove_file(&recovered_info)?;
                        sync_directory(
                            recovered_info
                                .parent()
                                .ok_or_else(|| invalid_data("Trash info parent is missing"))?,
                        )?;
                        record.resolution_info_identity = None;
                        self.persist(record_path, record, false)?;
                    }
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        }
        if !resolution_primary_tree_matches(record)?
            || !record_info_matches(record)?
            || entry_exists(&delete_path)?
            || !resolution_tree_matches(record, &recovered_data, true)?
            || !recovered_resolution_info_matches(record)?
        {
            return Ok(false);
        }
        self.finish_record(record_path)?;
        Ok(true)
    }

    fn recover_rebuilt_metadata(
        &self,
        record_path: &Path,
        record: &mut TrashRecord,
    ) -> io::Result<bool> {
        let return_staged = match record.resolution_intent {
            Some(TrashResolutionIntent::RebuildMetadata) => false,
            Some(TrashResolutionIntent::ReturnDeleteStageAndRebuildMetadata) => true,
            _ => return Err(invalid_data("record has no metadata-rebuild intent")),
        };
        let data = record.data_path();
        if return_staged {
            let delete = record
                .delete_path()
                .ok_or_else(|| invalid_data("metadata recovery has no delete stage"))?;
            if entry_exists(&delete)?
                && !entry_exists(&data)?
                && resolution_tree_matches(record, &delete, false)?
            {
                rename_noreplace(&delete, &data)?;
                sync_directory(
                    data.parent()
                        .ok_or_else(|| invalid_data("Trash data parent is missing"))?,
                )?;
            }
            if entry_exists(&delete)? || !resolution_tree_matches(record, &data, true)? {
                return Ok(false);
            }
        } else if !resolution_tree_matches(record, &data, false)? {
            return Ok(false);
        }

        let expected_sha = record
            .resolution_info_sha256
            .ok_or_else(|| invalid_data("metadata recovery has no digest"))?;
        let info_identity = match capture_optional_info(&record.info_path())? {
            Some(snapshot)
                if snapshot.sha256 == expected_sha
                    && record
                        .resolution_info_identity
                        .as_ref()
                        .is_none_or(|expected| expected == &snapshot.identity) =>
            {
                snapshot.identity
            }
            Some(_) => return Ok(false),
            None => {
                let bytes = record
                    .resolution_info_bytes
                    .as_ref()
                    .ok_or_else(|| invalid_data("metadata recovery has no bytes"))?;
                create_info_file(&record.info_path(), bytes)?
            }
        };
        record.resolution_info_identity = Some(info_identity.clone());
        let complete_trash = record.operation == TrashOperation::Trash
            && !entry_exists(&record.source())?
            && record_data_matches_after_rename(record)?;
        if record.operation != TrashOperation::Trash
            || record.stage != TrashStage::Prepared
            || complete_trash
        {
            record.info_identity = Some(info_identity);
            record.info_sha256 = expected_sha;
        }
        if complete_trash {
            record.stage = TrashStage::DataMoved;
        }
        self.persist(record_path, record, false)?;
        if !resolution_info_matches(record)? {
            return Ok(false);
        }
        self.undo
            .discard_trash_item(&record.data_path(), &record.info_path())?;
        if complete_trash {
            self.finish_undoable_record(record_path, record)?;
        } else {
            self.finish_record(record_path)?;
        }
        Ok(true)
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn trash(&self, source: &Path, cancel: &AtomicBool) -> io::Result<()> {
        let _lock = self.lock()?;
        let recovery = self.recover_locked()?;
        if recovery.pending != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "unfinished Trash recovery must be reviewed first",
            ));
        }
        let source = canonical_source_path(source)?;
        let layout = resolve_layout(&source)?;
        self.trash_in_layout(&source, &layout, Local::now(), Some(cancel))
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn restore(&self, item: &TrashedItem, cancel: &AtomicBool) -> io::Result<PathBuf> {
        let _lock = self.lock()?;
        let recovery = self.recover_locked()?;
        if recovery.pending != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "unfinished Trash recovery must be reviewed first",
            ));
        }
        validate_existing_layout(&item.layout)?;
        let original_name = item
            .original_path
            .file_name()
            .ok_or_else(|| invalid_data("restore identity has no file name"))?;
        let original_parent = item
            .original_path
            .parent()
            .ok_or_else(|| invalid_data("restore identity has no parent"))?;
        let canonical_parent = fs::canonicalize(original_parent)?;
        let destination = canonical_parent.join(original_name);
        if !path_is_normal_absolute(&destination)
            || destination.starts_with(&item.layout.root)
            || item.layout.root.starts_with(&destination)
        {
            return Err(invalid_data("restore destination is invalid"));
        }
        let parent_metadata = fs::symlink_metadata(&canonical_parent)?;
        if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
            return Err(invalid_data(
                "restore destination parent is not a real directory",
            ));
        }
        if entry_exists(&destination)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "an item already exists at the restore destination",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let record = TrashRecord {
            version: RECORD_VERSION,
            id: id.clone(),
            operation: TrashOperation::Restore,
            stage: TrashStage::RestorePrepared,
            source_path_bytes: destination.as_os_str().as_bytes().to_vec(),
            trash_root_path_bytes: item.layout.root.as_os_str().as_bytes().to_vec(),
            data_path_bytes: item.data_path.as_os_str().as_bytes().to_vec(),
            info_path_bytes: item.info_path.as_os_str().as_bytes().to_vec(),
            source_identity: item.data_identity.clone(),
            source_manifest: item.data_manifest.clone(),
            info_sha256: item.info_sha256,
            info_identity: Some(item.info_identity.clone()),
            restore_parent_identity: Some(EntryIdentity::capture(&canonical_parent)?),
            delete_path_bytes: None,
            resolution_intent: None,
            resolution_identity: None,
            resolution_manifest: None,
            resolution_info_identity: None,
            resolution_info_sha256: None,
            resolution_info_bytes: None,
            resolution_primary_identity: None,
            resolution_primary_manifest: None,
            resolution_primary_info_identity: None,
            resolution_data_path_bytes: None,
            resolution_info_path_bytes: None,
            create_undo_receipt: true,
            trash_source_parent_identity: None,
        };
        record.validate(&id)?;
        if !record_data_matches_exact(&record)?
            || !record_info_matches(&record)?
            || !restore_parent_matches(&record, true)?
        {
            return Err(changed("Trash item changed before restore"));
        }
        let record_path = self.record_path(&id);
        self.persist(&record_path, &record, true)?;
        if cancel.load(Ordering::Acquire) {
            self.finish_record(&record_path)?;
            return Err(interrupted_restore());
        }
        if entry_exists(&destination)?
            || !record_data_matches_exact(&record)?
            || !record_info_matches(&record)?
            || !restore_parent_matches(&record, true)?
        {
            return Err(changed("Trash item or restore destination changed"));
        }
        let mut record = record;
        if !self.resume_restore_prepared(&record_path, &mut record)? {
            return Err(changed("restored data changed during publication"));
        }
        Ok(destination)
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn delete_permanently(
        &self,
        item: &TrashedItem,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        let recovery = self.recover_locked()?;
        if recovery.pending != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "unfinished Trash recovery must be reviewed first",
            ));
        }
        validate_existing_layout(&item.layout)?;
        let id = Uuid::new_v4().to_string();
        let delete_path = item
            .data_path
            .parent()
            .ok_or_else(|| invalid_data("Trash data parent is missing"))?
            .join(format!(".rmac-delete-{id}"));
        let record = TrashRecord {
            version: RECORD_VERSION,
            id: id.clone(),
            operation: TrashOperation::Delete,
            stage: TrashStage::DeletePrepared,
            source_path_bytes: item.original_path.as_os_str().as_bytes().to_vec(),
            trash_root_path_bytes: item.layout.root.as_os_str().as_bytes().to_vec(),
            data_path_bytes: item.data_path.as_os_str().as_bytes().to_vec(),
            info_path_bytes: item.info_path.as_os_str().as_bytes().to_vec(),
            source_identity: item.data_identity.clone(),
            source_manifest: item.data_manifest.clone(),
            info_sha256: item.info_sha256,
            info_identity: Some(item.info_identity.clone()),
            restore_parent_identity: None,
            delete_path_bytes: Some(delete_path.as_os_str().as_bytes().to_vec()),
            resolution_intent: None,
            resolution_identity: None,
            resolution_manifest: None,
            resolution_info_identity: None,
            resolution_info_sha256: None,
            resolution_info_bytes: None,
            resolution_primary_identity: None,
            resolution_primary_manifest: None,
            resolution_primary_info_identity: None,
            resolution_data_path_bytes: None,
            resolution_info_path_bytes: None,
            create_undo_receipt: false,
            trash_source_parent_identity: None,
        };
        record.validate(&id)?;
        if !record_data_matches_exact(&record)?
            || !record_info_matches(&record)?
            || entry_exists(&delete_path)?
        {
            return Err(changed("Trash item changed before permanent deletion"));
        }
        let record_path = self.record_path(&id);
        self.persist(&record_path, &record, true)?;
        if cancel.load(Ordering::Acquire) {
            self.finish_record(&record_path)?;
            return Err(interrupted_delete());
        }
        if !record_data_matches_exact(&record)?
            || !record_info_matches(&record)?
            || entry_exists(&delete_path)?
        {
            return Err(changed("Trash item changed before permanent deletion"));
        }
        let mut record = record;
        if !self.resume_delete_prepared(&record_path, &mut record)? {
            return Err(changed("Trash item changed during permanent deletion"));
        }
        Ok(())
    }

    fn trash_in_layout(
        &self,
        source: &Path,
        layout: &TrashLayout,
        deleted_at: DateTime<Local>,
        cancel: Option<&AtomicBool>,
    ) -> io::Result<()> {
        validate_source_path(source)?;
        if source.starts_with(&layout.root) || layout.root.starts_with(source) {
            return Err(invalid_input(
                "Trash source and Trash storage must not contain one another",
            ));
        }
        ensure_trash_layout(layout)?;
        let source_identity = EntryIdentity::capture(source)?;
        let source_manifest = match cancel {
            Some(cancel) => TreeManifest::capture_cancellable(source, cancel)?,
            None => TreeManifest::capture(source)?,
        };
        if EntryIdentity::capture(source)? != source_identity {
            return Err(changed("source changed while preparing Trash"));
        }
        let original_name = source
            .file_name()
            .ok_or_else(|| invalid_input("Trash source has no file name"))?;

        for suffix in 1..=10_000usize {
            let trash_name = suffixed_name(original_name, suffix);
            let data_path = layout.root.join("files").join(&trash_name);
            let mut info_name = trash_name.clone();
            info_name.push(TRASHINFO_SUFFIX);
            let info_path = layout.root.join("info").join(info_name);
            if entry_exists(&data_path)? || entry_exists(&info_path)? {
                continue;
            }
            let info_bytes = trash_info_bytes(source, layout, deleted_at)?;
            let id = Uuid::new_v4().to_string();
            let mut record = TrashRecord {
                version: RECORD_VERSION,
                id: id.clone(),
                operation: TrashOperation::Trash,
                stage: TrashStage::Prepared,
                source_path_bytes: source.as_os_str().as_bytes().to_vec(),
                trash_root_path_bytes: layout.root.as_os_str().as_bytes().to_vec(),
                data_path_bytes: data_path.as_os_str().as_bytes().to_vec(),
                info_path_bytes: info_path.as_os_str().as_bytes().to_vec(),
                source_identity: source_identity.clone(),
                source_manifest: source_manifest.clone(),
                info_sha256: sha256(&info_bytes),
                info_identity: None,
                restore_parent_identity: None,
                delete_path_bytes: None,
                resolution_intent: None,
                resolution_identity: None,
                resolution_manifest: None,
                resolution_info_identity: None,
                resolution_info_sha256: None,
                resolution_info_bytes: None,
                resolution_primary_identity: None,
                resolution_primary_manifest: None,
                resolution_primary_info_identity: None,
                resolution_data_path_bytes: None,
                resolution_info_path_bytes: None,
                create_undo_receipt: true,
                trash_source_parent_identity: Some(EntryIdentity::capture(
                    source
                        .parent()
                        .ok_or_else(|| invalid_data("Trash source parent is missing"))?,
                )?),
            };
            record.validate(&id)?;
            let record_path = self.record_path(&id);
            self.persist(&record_path, &record, true)?;
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
                self.finish_record(&record_path)?;
                return Err(interrupted());
            }

            match create_info_file(&info_path, &info_bytes) {
                Ok(identity) => {
                    record.info_identity = Some(identity);
                    record.stage = TrashStage::InfoPublished;
                    self.persist(&record_path, &record, false)?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    self.finish_record(&record_path)?;
                    continue;
                }
                Err(error) => return Err(error),
            }

            if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
                remove_exact_info(&record)?;
                self.finish_record(&record_path)?;
                return Err(interrupted());
            }
            let source_matches = match record_source_matches_with_cancel(&record, cancel) {
                Ok(matches) => matches,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    remove_exact_info(&record)?;
                    self.finish_record(&record_path)?;
                    return Err(interrupted());
                }
                Err(error) => return Err(error),
            };
            if !source_matches {
                return Err(changed("source changed before it could be moved to Trash"));
            }
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
                remove_exact_info(&record)?;
                self.finish_record(&record_path)?;
                return Err(interrupted());
            }
            match rename_noreplace(source, &data_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    remove_exact_info(&record)?;
                    self.finish_record(&record_path)?;
                    continue;
                }
                Err(error) => return Err(error),
            }
            sync_directory(
                data_path
                    .parent()
                    .ok_or_else(|| invalid_data("Trash data parent is missing"))?,
            )?;
            sync_directory(
                source
                    .parent()
                    .ok_or_else(|| invalid_data("Trash source parent is missing"))?,
            )?;
            if !record_data_matches_after_rename(&record)? {
                return Err(changed("trashed data changed during publication"));
            }
            record.stage = TrashStage::DataMoved;
            self.persist(&record_path, &record, false)?;
            self.finish_undoable_record(&record_path, &record)?;
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no unique Trash name is available",
        ))
    }

    fn recover_locked(&self) -> io::Result<TrashRecovery> {
        let mut report = TrashRecovery::default();
        for (path, mut record) in self.read_records()? {
            let finalized = match record.resolution_intent {
                Some(
                    TrashResolutionIntent::RebuildMetadata
                    | TrashResolutionIntent::ReturnDeleteStageAndRebuildMetadata,
                ) => self.recover_rebuilt_metadata(&path, &mut record)?,
                Some(TrashResolutionIntent::PreserveConflictingItems) => {
                    self.recover_conflicting_items(&path, &mut record)?
                }
                _ => match record.operation {
                    TrashOperation::Trash => {
                        let source_exists = entry_exists(&record.source())?;
                        let data_exists = entry_exists(&record.data_path())?;
                        let info_exists = entry_exists(&record.info_path())?;
                        let source_matches = source_exists && record_source_matches(&record)?;
                        let data_matches =
                            data_exists && record_data_matches_after_rename(&record)?;
                        let info_matches = info_exists && record_info_matches(&record)?;
                        match record.stage {
                            TrashStage::Prepared
                                if source_matches && !data_exists && !info_exists =>
                            {
                                self.finish_record(&path)?;
                                true
                            }
                            TrashStage::Prepared
                                if source_matches && !data_exists && info_matches =>
                            {
                                record.info_identity =
                                    Some(EntryIdentity::capture(&record.info_path())?);
                                record.stage = TrashStage::InfoPublished;
                                self.persist(&path, &record, false)?;
                                self.resume_info_published(&path, &mut record)?
                            }
                            TrashStage::InfoPublished
                                if source_matches && !data_exists && info_matches =>
                            {
                                self.resume_info_published(&path, &mut record)?
                            }
                            TrashStage::InfoPublished | TrashStage::DataMoved
                                if !source_exists && data_matches && info_matches =>
                            {
                                self.finish_undoable_record(&path, &record)?;
                                true
                            }
                            _ => false,
                        }
                    }
                    TrashOperation::Restore => self.recover_restore_record(&path, &mut record)?,
                    TrashOperation::Delete => self.recover_delete_record(&path, &mut record)?,
                },
            };
            if finalized {
                report.finalized += 1;
            } else {
                report.pending += 1;
            }
        }
        Ok(report)
    }

    fn recover_restore_record(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        let data_exists = entry_exists(&record.data_path())?;
        let destination_exists = entry_exists(&record.source())?;
        let info_exists = entry_exists(&record.info_path())?;
        let data_matches = data_exists && record_data_matches_exact(record)?;
        let destination_matches = destination_exists && record_restored_data_matches(record)?;
        let info_matches = info_exists && record_info_matches(record)?;
        let parent_exact = restore_parent_matches(record, true)?;
        let parent_same = restore_parent_matches(record, false)?;

        match record.stage {
            TrashStage::RestorePrepared
                if data_matches && !destination_exists && info_matches && parent_exact =>
            {
                self.resume_restore_prepared(path, record)
            }
            TrashStage::RestorePrepared
                if !data_exists && destination_matches && info_matches && parent_same =>
            {
                record.stage = TrashStage::RestoreDataMoved;
                self.persist(path, record, false)?;
                self.finish_restore_info(path, record)
            }
            TrashStage::RestoreDataMoved
                if !data_exists
                    && destination_matches
                    && parent_same
                    && (!info_exists || info_matches) =>
            {
                self.finish_restore_info(path, record)
            }
            TrashStage::RestoreInfoRemoved
                if !data_exists && destination_matches && !info_exists && parent_same =>
            {
                self.finish_undoable_record(path, record)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn recover_delete_record(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        if record.resolution_intent == Some(TrashResolutionIntent::ReturnDeleteStage) {
            return self.recover_returned_delete_stage(path, record);
        }
        let delete_path = record
            .delete_path()
            .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
        let data_exists = entry_exists(&record.data_path())?;
        let delete_exists = entry_exists(&delete_path)?;
        let info_exists = entry_exists(&record.info_path())?;
        let data_matches = data_exists && record_data_matches_exact(record)?;
        let delete_matches = delete_exists && record_delete_data_matches_exact(record)?;
        let delete_directory_matches =
            delete_exists && record_delete_directory_root_matches(record)?;
        let info_matches = info_exists && record_info_matches(record)?;

        match record.stage {
            TrashStage::DeletePrepared if data_matches && !delete_exists && info_matches => {
                self.resume_delete_prepared(path, record)
            }
            TrashStage::DeletePrepared if !data_exists && delete_matches && info_matches => {
                record.stage = TrashStage::DeleteDataStaged;
                self.persist(path, record, false)?;
                self.resume_delete_staged(path, record, false)
            }
            TrashStage::DeleteDataStaged
                if !data_exists
                    && info_matches
                    && (!delete_exists || delete_matches || delete_directory_matches) =>
            {
                self.resume_delete_staged(path, record, true)
            }
            TrashStage::DeleteDataRemoved
                if !data_exists && !delete_exists && (!info_exists || info_matches) =>
            {
                self.finish_delete_info(path, record)
            }
            TrashStage::DeleteInfoRemoved if !data_exists && !delete_exists && !info_exists => {
                self.undo
                    .discard_trash_item(&record.data_path(), &record.info_path())?;
                self.finish_record(path)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn recover_returned_delete_stage(
        &self,
        path: &Path,
        record: &mut TrashRecord,
    ) -> io::Result<bool> {
        let delete_path = record
            .delete_path()
            .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
        let delete_exists = entry_exists(&delete_path)?;
        let data_exists = entry_exists(&record.data_path())?;
        if !resolution_info_matches(record)? {
            return Ok(false);
        }
        if delete_exists && !data_exists && resolution_tree_matches(record, &delete_path, false)? {
            rename_noreplace(&delete_path, &record.data_path())?;
            sync_directory(
                record
                    .data_path()
                    .parent()
                    .ok_or_else(|| invalid_data("Trash data parent is missing"))?,
            )?;
            if !resolution_tree_matches(record, &record.data_path(), true)? {
                return Ok(false);
            }
            self.finish_record(path)?;
            return Ok(true);
        }
        if !delete_exists
            && data_exists
            && resolution_tree_matches(record, &record.data_path(), true)?
        {
            self.finish_record(path)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn resume_info_published(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        rename_noreplace(&record.source(), &record.data_path())?;
        sync_directory(
            record
                .data_path()
                .parent()
                .ok_or_else(|| invalid_data("Trash data parent is missing"))?,
        )?;
        sync_directory(
            record
                .source()
                .parent()
                .ok_or_else(|| invalid_data("Trash source parent is missing"))?,
        )?;
        if !record_data_matches_after_rename(record)? {
            return Ok(false);
        }
        record.stage = TrashStage::DataMoved;
        self.persist(path, record, false)?;
        self.finish_undoable_record(path, record)?;
        Ok(true)
    }

    fn resume_restore_prepared(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        rename_noreplace(&record.data_path(), &record.source())?;
        sync_directory(
            record
                .data_path()
                .parent()
                .ok_or_else(|| invalid_data("Trash data parent is missing"))?,
        )?;
        sync_directory(
            record
                .source()
                .parent()
                .ok_or_else(|| invalid_data("restore destination parent is missing"))?,
        )?;
        if !record_restored_data_matches(record)? {
            return Ok(false);
        }
        record.stage = TrashStage::RestoreDataMoved;
        self.persist(path, record, false)?;
        self.finish_restore_info(path, record)
    }

    fn finish_restore_info(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        if entry_exists(&record.info_path())? {
            if !record_info_matches(record)? {
                return Ok(false);
            }
            self.archive_undo_receipt(path, record)?;
            fs::remove_file(record.info_path())?;
            sync_directory(
                record
                    .info_path()
                    .parent()
                    .ok_or_else(|| invalid_data("Trash info parent is missing"))?,
            )?;
        } else if record.create_undo_receipt && !self.undo.has_receipt(&record.id)? {
            return Ok(false);
        }
        record.stage = TrashStage::RestoreInfoRemoved;
        self.persist(path, record, false)?;
        self.finish_undoable_record(path, record)?;
        Ok(true)
    }

    fn resume_delete_prepared(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        let delete_path = record
            .delete_path()
            .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
        rename_noreplace(&record.data_path(), &delete_path)?;
        sync_directory(
            delete_path
                .parent()
                .ok_or_else(|| invalid_data("permanent-delete staging parent is missing"))?,
        )?;
        if !record_delete_data_matches_exact(record)? {
            return Ok(false);
        }
        record.stage = TrashStage::DeleteDataStaged;
        self.persist(path, record, false)?;
        self.resume_delete_staged(path, record, false)
    }

    fn resume_delete_staged(
        &self,
        path: &Path,
        record: &mut TrashRecord,
        allow_partial_directory: bool,
    ) -> io::Result<bool> {
        let delete_path = record
            .delete_path()
            .ok_or_else(|| invalid_data("delete transaction has no staging path"))?;
        if entry_exists(&delete_path)? {
            let exact = record_delete_data_matches_exact(record)?;
            let partial_directory =
                allow_partial_directory && record_delete_directory_root_matches(record)?;
            if !exact && !partial_directory {
                return Ok(false);
            }
            remove_staged_delete_data(record)?;
        }
        if entry_exists(&delete_path)? || entry_exists(&record.data_path())? {
            return Ok(false);
        }
        record.stage = TrashStage::DeleteDataRemoved;
        self.persist(path, record, false)?;
        self.finish_delete_info(path, record)
    }

    fn finish_delete_info(&self, path: &Path, record: &mut TrashRecord) -> io::Result<bool> {
        self.undo
            .discard_trash_item(&record.data_path(), &record.info_path())?;
        if entry_exists(&record.info_path())? {
            if !record_info_matches(record)? {
                return Ok(false);
            }
            fs::remove_file(record.info_path())?;
            sync_directory(
                record
                    .info_path()
                    .parent()
                    .ok_or_else(|| invalid_data("Trash info parent is missing"))?,
            )?;
        }
        record.stage = TrashStage::DeleteInfoRemoved;
        self.persist(path, record, false)?;
        self.finish_record(path)?;
        Ok(true)
    }

    fn lock(&self) -> io::Result<File> {
        let path = self.state_root.join(".lock");
        let descriptor = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(io::Error::from)?;
        let file = File::from(descriptor);
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.mode() & 0o777 != 0o600
            || metadata.uid() != effective_uid()
            || metadata.len() != 0
        {
            return Err(invalid_data(
                "Trash transaction lock is not a private empty regular file",
            ));
        }
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(io::Error::from)?;
        Ok(file)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.state_root.join(format!("{id}.json"))
    }

    fn persist(&self, path: &Path, record: &TrashRecord, create: bool) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(record).map_err(invalid_json)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(invalid_data("Trash transaction record is too large"));
        }
        let temp = self
            .state_root
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
            sync_directory(&self.state_root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    fn finish_record(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)?;
        sync_directory(&self.state_root)
    }

    fn finish_undoable_record(&self, path: &Path, record: &TrashRecord) -> io::Result<()> {
        if !record.create_undo_receipt {
            return self.finish_record(path);
        }
        self.archive_undo_receipt(path, record)?;
        self.finish_record(path)?;
        self.undo.activate(&record.id)
    }

    fn archive_undo_receipt(&self, path: &Path, record: &TrashRecord) -> io::Result<()> {
        if !record.create_undo_receipt {
            return Ok(());
        }
        let kind = match record.operation {
            TrashOperation::Trash => UndoKind::Trash,
            TrashOperation::Restore => UndoKind::Restore,
            TrashOperation::Delete => {
                return Err(invalid_data(
                    "permanent deletion cannot create an Undo receipt",
                ));
            }
        };
        let info_bytes = match read_bound_info_bytes(record)? {
            Some(bytes) => bytes,
            None if kind == UndoKind::Restore && self.undo.has_receipt(&record.id)? => {
                return Ok(());
            }
            None => {
                return Err(changed(
                    "Trash metadata disappeared before Undo could be archived",
                ));
            }
        };
        let source_parent_identity = match kind {
            UndoKind::Trash => record
                .trash_source_parent_identity
                .clone()
                .ok_or_else(|| invalid_data("Move-to-Trash Undo has no source parent"))?,
            UndoKind::Restore => record
                .restore_parent_identity
                .clone()
                .ok_or_else(|| invalid_data("Restore Undo has no source parent"))?,
            _ => unreachable!("Trash operation kind was checked above"),
        };
        let item_snapshot = TransferTreeSnapshot::from_parts(
            record.source_identity.clone(),
            record.source_manifest.clone(),
        );
        self.undo.archive_trash(TrashUndoSeed {
            id: record.id.clone(),
            kind,
            source: record.source(),
            data: record.data_path(),
            info: record.info_path(),
            source_parent_identity,
            item_snapshot,
            info_bytes,
            info_identity: (kind == UndoKind::Trash)
                .then(|| record.info_identity.clone())
                .flatten(),
            forward_record: path.to_path_buf(),
        })
    }

    fn read_records(&self) -> io::Result<Vec<(PathBuf, TrashRecord)>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.state_root)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid_data("Trash state name is not UTF-8"))?;
            if name == ".lock" {
                continue;
            }
            if name.starts_with('.') && name.ends_with(".tmp") {
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.mode() & 0o777 != 0o600
                    || metadata.uid() != effective_uid()
                    || metadata.len() > MAX_RECORD_BYTES
                {
                    return Err(invalid_data(
                        "Trash temporary is not a bounded private regular file",
                    ));
                }
                fs::remove_file(path)?;
                continue;
            }
            if !name.ends_with(".json") {
                return Err(invalid_data("unexpected entry in Trash transaction state"));
            }
            paths.push(path);
            if paths.len() > MAX_RECORDS {
                return Err(invalid_data("too many unfinished Trash transactions"));
            }
        }
        paths.sort();
        let mut records = Vec::with_capacity(paths.len());
        for path in paths {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.mode() & 0o777 != 0o600
                || metadata.uid() != effective_uid()
                || metadata.len() > MAX_RECORD_BYTES
            {
                return Err(invalid_data(
                    "Trash transaction is not a bounded private regular file",
                ));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            File::open(&path)?
                .take(MAX_RECORD_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_RECORD_BYTES {
                return Err(invalid_data("Trash transaction record is too large"));
            }
            let record: TrashRecord = serde_json::from_slice(&bytes).map_err(invalid_json)?;
            let id = path
                .file_stem()
                .and_then(OsStr::to_str)
                .ok_or_else(|| invalid_data("Trash transaction file name is invalid"))?;
            record.validate(id)?;
            records.push((path, record));
        }
        Ok(records)
    }
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
                "rmac-files-trash-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory should be created");
            Self(fs::canonicalize(path).expect("test directory should canonicalize"))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn deleted_at() -> DateTime<Local> {
        DateTime::parse_from_rfc3339("2026-07-29T08:09:10+05:30")
            .expect("fixture date should parse")
            .with_timezone(&Local)
    }

    fn setup(label: &str) -> (TestDirectory, TrashStore, TrashLayout) {
        let directory = TestDirectory::new(label);
        let store = TrashStore::open(directory.0.join("state"))
            .expect("Trash store should open in a private directory");
        let layout = TrashLayout {
            topdir: directory.0.clone(),
            root: directory.0.join("Trash"),
            path_relative_to_topdir: true,
        };
        (directory, store, layout)
    }

    fn write_source(directory: &TestDirectory, name: &OsStr, contents: &[u8]) -> PathBuf {
        let source = directory.0.join(name);
        fs::write(&source, contents).expect("source fixture should be written");
        source
    }

    fn prepared_record(
        store: &TrashStore,
        layout: &TrashLayout,
        source: &Path,
    ) -> (PathBuf, TrashRecord, Vec<u8>) {
        ensure_trash_layout(layout).expect("Trash layout should be created");
        let name = source
            .file_name()
            .expect("source fixture should have a name")
            .to_os_string();
        let data_path = layout.root.join("files").join(&name);
        let mut info_name = name;
        info_name.push(TRASHINFO_SUFFIX);
        let info_path = layout.root.join("info").join(info_name);
        let info_bytes =
            trash_info_bytes(source, layout, deleted_at()).expect("Trash metadata should encode");
        let id = Uuid::new_v4().to_string();
        let record = TrashRecord {
            version: RECORD_VERSION,
            id: id.clone(),
            operation: TrashOperation::Trash,
            stage: TrashStage::Prepared,
            source_path_bytes: source.as_os_str().as_bytes().to_vec(),
            trash_root_path_bytes: layout.root.as_os_str().as_bytes().to_vec(),
            data_path_bytes: data_path.as_os_str().as_bytes().to_vec(),
            info_path_bytes: info_path.as_os_str().as_bytes().to_vec(),
            source_identity: EntryIdentity::capture(source)
                .expect("source identity should be captured"),
            source_manifest: TreeManifest::capture(source)
                .expect("source manifest should be captured"),
            info_sha256: sha256(&info_bytes),
            info_identity: None,
            restore_parent_identity: None,
            delete_path_bytes: None,
            resolution_intent: None,
            resolution_identity: None,
            resolution_manifest: None,
            resolution_info_identity: None,
            resolution_info_sha256: None,
            resolution_info_bytes: None,
            resolution_primary_identity: None,
            resolution_primary_manifest: None,
            resolution_primary_info_identity: None,
            resolution_data_path_bytes: None,
            resolution_info_path_bytes: None,
            create_undo_receipt: false,
            trash_source_parent_identity: None,
        };
        let record_path = store.record_path(&id);
        store
            .persist(&record_path, &record, true)
            .expect("prepared transaction should persist");
        (record_path, record, info_bytes)
    }

    fn trash_and_list(store: &TrashStore, layout: &TrashLayout, source: &Path) -> TrashedItem {
        store
            .trash_in_layout(source, layout, deleted_at(), None)
            .expect("fixture should move to Trash");
        let mut items = list_in_layout(layout).expect("fixture Trash should list");
        assert_eq!(items.len(), 1);
        items.remove(0)
    }

    fn prepared_restore_record(store: &TrashStore, item: &TrashedItem) -> (PathBuf, TrashRecord) {
        let parent = fs::canonicalize(item.original_path.parent().unwrap()).unwrap();
        let destination = parent.join(item.original_path.file_name().unwrap());
        let id = Uuid::new_v4().to_string();
        let record = TrashRecord {
            version: RECORD_VERSION,
            id: id.clone(),
            operation: TrashOperation::Restore,
            stage: TrashStage::RestorePrepared,
            source_path_bytes: destination.as_os_str().as_bytes().to_vec(),
            trash_root_path_bytes: item.layout.root.as_os_str().as_bytes().to_vec(),
            data_path_bytes: item.data_path.as_os_str().as_bytes().to_vec(),
            info_path_bytes: item.info_path.as_os_str().as_bytes().to_vec(),
            source_identity: item.data_identity.clone(),
            source_manifest: item.data_manifest.clone(),
            info_sha256: item.info_sha256,
            info_identity: Some(item.info_identity.clone()),
            restore_parent_identity: Some(EntryIdentity::capture(&parent).unwrap()),
            delete_path_bytes: None,
            resolution_intent: None,
            resolution_identity: None,
            resolution_manifest: None,
            resolution_info_identity: None,
            resolution_info_sha256: None,
            resolution_info_bytes: None,
            resolution_primary_identity: None,
            resolution_primary_manifest: None,
            resolution_primary_info_identity: None,
            resolution_data_path_bytes: None,
            resolution_info_path_bytes: None,
            create_undo_receipt: false,
            trash_source_parent_identity: None,
        };
        let record_path = store.record_path(&id);
        store.persist(&record_path, &record, true).unwrap();
        (record_path, record)
    }

    fn prepared_delete_record(store: &TrashStore, item: &TrashedItem) -> (PathBuf, TrashRecord) {
        let id = Uuid::new_v4().to_string();
        let delete_path = item
            .data_path
            .parent()
            .unwrap()
            .join(format!(".rmac-delete-{id}"));
        let record = TrashRecord {
            version: RECORD_VERSION,
            id: id.clone(),
            operation: TrashOperation::Delete,
            stage: TrashStage::DeletePrepared,
            source_path_bytes: item.original_path.as_os_str().as_bytes().to_vec(),
            trash_root_path_bytes: item.layout.root.as_os_str().as_bytes().to_vec(),
            data_path_bytes: item.data_path.as_os_str().as_bytes().to_vec(),
            info_path_bytes: item.info_path.as_os_str().as_bytes().to_vec(),
            source_identity: item.data_identity.clone(),
            source_manifest: item.data_manifest.clone(),
            info_sha256: item.info_sha256,
            info_identity: Some(item.info_identity.clone()),
            restore_parent_identity: None,
            delete_path_bytes: Some(delete_path.as_os_str().as_bytes().to_vec()),
            resolution_intent: None,
            resolution_identity: None,
            resolution_manifest: None,
            resolution_info_identity: None,
            resolution_info_sha256: None,
            resolution_info_bytes: None,
            resolution_primary_identity: None,
            resolution_primary_manifest: None,
            resolution_primary_info_identity: None,
            resolution_data_path_bytes: None,
            resolution_info_path_bytes: None,
            create_undo_receipt: false,
            trash_source_parent_identity: None,
        };
        let record_path = store.record_path(&id);
        store.persist(&record_path, &record, true).unwrap();
        (record_path, record)
    }

    #[test]
    fn trash_moves_data_and_publishes_freedesktop_identity_durably() {
        let (directory, store, layout) = setup("success");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");

        store
            .trash_in_layout(&source, &layout, deleted_at(), None)
            .expect("file should move to Trash");
        let data_path = layout.root.join("files/report.txt");
        let info_path = layout.root.join("info/report.txt.trashinfo");

        assert!(!source.exists());
        assert_eq!(fs::read(data_path).unwrap(), b"important");
        assert_eq!(
            fs::read_to_string(info_path).unwrap(),
            "[Trash Info]\nPath=report.txt\nDeletionDate=2026-07-29T08:09:10\n"
        );
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn completed_trash_is_the_latest_durable_undo_and_restores_exactly() {
        let (directory, store, layout) = setup("trash-undo");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);

        let available = store.undo_store().latest().unwrap().unwrap();
        assert_eq!(available.label, "Undo Move to Trash “report.txt”");
        assert!(available.uses_trash);
        assert_eq!(store.undo_store().count().unwrap(), 1);

        let outcome = store
            .execute_latest_undo(
                &crate::file_ops::RealFileSystem,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap()
            .unwrap();

        assert_eq!(outcome.label, "Undo Move to Trash “report.txt”");
        assert_eq!(fs::read(&source).unwrap(), b"important");
        assert!(!item.data_path.exists());
        assert!(!item.info_path.exists());
        assert_eq!(store.undo_store().count().unwrap(), 0);
    }

    #[test]
    fn completed_restore_undo_recreates_exact_metadata_and_returns_item_to_trash() {
        let (directory, store, layout) = setup("restore-undo");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let info_bytes = fs::read(&item.info_path).unwrap();

        store
            .restore(&item, &AtomicBool::new(false))
            .expect("restore should complete");

        let available = store.undo_store().latest().unwrap().unwrap();
        assert_eq!(available.label, "Undo Restore “report.txt”");
        assert!(source.exists());
        assert!(!item.data_path.exists());
        assert!(!item.info_path.exists());

        store
            .execute_latest_undo(
                &crate::file_ops::RealFileSystem,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap()
            .unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert_eq!(fs::read(&item.info_path).unwrap(), info_bytes);
        assert_eq!(list_in_layout(&layout).unwrap().len(), 1);
    }

    #[test]
    fn trash_undo_never_replaces_a_racing_original_location() {
        let (directory, store, layout) = setup("trash-undo-source-race");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(&source, b"racing item").unwrap();

        let error = store
            .execute_latest_undo(
                &crate::file_ops::RealFileSystem,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&source).unwrap(), b"racing item");
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert!(item.info_path.exists());
        assert_eq!(store.undo_store().count().unwrap(), 1);
    }

    #[test]
    fn changed_trash_data_is_never_removed_by_undo() {
        let (directory, store, layout) = setup("trash-undo-data-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(&item.data_path, b"changed").unwrap();

        let error = store
            .execute_latest_undo(
                &crate::file_ops::RealFileSystem,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!source.exists());
        assert_eq!(fs::read(&item.data_path).unwrap(), b"changed");
        assert!(item.info_path.exists());
    }

    #[test]
    fn changed_trash_metadata_is_never_removed_by_undo() {
        let (directory, store, layout) = setup("trash-undo-info-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::remove_file(&item.info_path).unwrap();
        fs::write(&item.info_path, b"[Trash Info]\nPath=/different\n").unwrap();

        let error = store
            .execute_latest_undo(
                &crate::file_ops::RealFileSystem,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!source.exists());
        assert!(item.data_path.exists());
        assert!(item.info_path.exists());
    }

    #[test]
    fn permanent_delete_discards_stale_trash_undo_without_creating_a_receipt() {
        let (directory, store, layout) = setup("delete-invalidates-undo");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        assert_eq!(store.undo_store().count().unwrap(), 1);

        store
            .delete_permanently(&item, &AtomicBool::new(false))
            .expect("permanent delete should complete");

        assert_eq!(store.undo_store().count().unwrap(), 0);
        assert!(store.undo_store().latest().unwrap().is_none());
        assert!(!source.exists());
        assert!(!item.data_path.exists());
        assert!(!item.info_path.exists());
    }

    #[test]
    fn listing_decodes_exact_restore_identity_without_debug_path_leak() {
        let (directory, store, layout) = setup("list");
        let source = write_source(&directory, OsStr::new("report 1.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);

        assert_eq!(item.name, OsStr::new("report 1.txt"));
        assert_eq!(item.original_path, source);
        assert_eq!(item.deleted_at, "2026-07-29T08:09:10");
        assert!(!format!("{item:?}").contains("report"));
    }

    #[test]
    fn listing_rejects_relative_identity_escape() {
        let (_directory, _store, layout) = setup("list-escape");
        ensure_trash_layout(&layout).unwrap();
        fs::write(layout.root.join("files/report.txt"), b"report").unwrap();
        create_info_file(
            &layout.root.join("info/report.txt.trashinfo"),
            b"[Trash Info]\nPath=../escape\nDeletionDate=2026-07-29T08:09:10\n",
        )
        .unwrap();

        let error = list_in_layout(&layout).expect_err("path traversal must fail closed");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn restore_moves_exact_data_back_and_removes_metadata() {
        let (directory, store, layout) = setup("restore");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let cancel = AtomicBool::new(false);

        let restored = store.restore(&item, &cancel).expect("item should restore");

        assert_eq!(restored, source);
        assert_eq!(fs::read(&source).unwrap(), b"important");
        assert!(!item.data_path.exists());
        assert!(!item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn restore_collision_preserves_both_items_without_record() {
        let (directory, store, layout) = setup("restore-collision");
        let source = write_source(&directory, OsStr::new("report.txt"), b"trashed");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(&source, b"existing").unwrap();

        let error = store
            .restore(&item, &AtomicBool::new(false))
            .expect_err("restore must not replace a collision");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&source).unwrap(), b"existing");
        assert_eq!(fs::read(&item.data_path).unwrap(), b"trashed");
        assert!(item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn restore_cancellation_keeps_trash_item_and_no_record() {
        let (directory, store, layout) = setup("restore-cancel");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let cancel = AtomicBool::new(true);

        let error = store
            .restore(&item, &cancel)
            .expect_err("cancelled restore should retain Trash item");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!source.exists());
        assert!(item.data_path.exists());
        assert!(item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn restore_refuses_data_substitution_after_listing() {
        let (directory, store, layout) = setup("restore-data-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(&item.data_path, b"changed").unwrap();

        let error = store
            .restore(&item, &AtomicBool::new(false))
            .expect_err("changed Trash data must not restore");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!source.exists());
        assert!(item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn restore_refuses_metadata_substitution_after_listing() {
        let (directory, store, layout) = setup("restore-info-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(
            &item.info_path,
            b"[Trash Info]\nPath=elsewhere\nDeletionDate=2026-07-29T08:09:10\n",
        )
        .unwrap();

        let error = store
            .restore(&item, &AtomicBool::new(false))
            .expect_err("changed Trash metadata must not restore");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!source.exists());
        assert!(item.data_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn recovery_finishes_restore_renamed_before_stage_persisted() {
        let (directory, store, layout) = setup("restore-rename-crash");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, record) = prepared_restore_record(&store, &item);
        rename_noreplace(&item.data_path, &record.source()).unwrap();
        sync_directory(record.source().parent().unwrap()).unwrap();
        sync_directory(item.data_path.parent().unwrap()).unwrap();

        let recovery = store.recover().expect("interrupted restore should recover");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert_eq!(fs::read(&source).unwrap(), b"important");
        assert!(!item.info_path.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_finishes_restore_after_metadata_removed() {
        let (directory, store, layout) = setup("restore-info-crash");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_restore_record(&store, &item);
        rename_noreplace(&item.data_path, &record.source()).unwrap();
        record.stage = TrashStage::RestoreDataMoved;
        store.persist(&record_path, &record, false).unwrap();
        fs::remove_file(&item.info_path).unwrap();
        sync_directory(item.info_path.parent().unwrap()).unwrap();

        let recovery = store
            .recover()
            .expect("metadata removal crash should recover");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(source.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_never_replaces_a_restore_destination_race() {
        let (directory, store, layout) = setup("restore-race");
        let source = write_source(&directory, OsStr::new("report.txt"), b"trashed");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, _) = prepared_restore_record(&store, &item);
        fs::write(&source, b"racing").unwrap();

        let recovery = store.recover().expect("collision should remain pending");

        assert_eq!(recovery.finalized, 0);
        assert_eq!(recovery.pending, 1);
        assert_eq!(fs::read(&source).unwrap(), b"racing");
        assert_eq!(fs::read(&item.data_path).unwrap(), b"trashed");
        assert!(item.info_path.exists());
        assert!(record_path.exists());
    }

    #[test]
    fn recovery_refuses_a_changed_restore_parent() {
        let (directory, store, layout) = setup("restore-parent-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"trashed");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, _) = prepared_restore_record(&store, &item);
        fs::write(directory.0.join("concurrent.txt"), b"change").unwrap();

        let recovery = store
            .recover()
            .expect("changed parent should remain pending");

        assert_eq!(recovery.finalized, 0);
        assert_eq!(recovery.pending, 1);
        assert!(!source.exists());
        assert!(item.data_path.exists());
        assert!(item.info_path.exists());
        assert!(record_path.exists());
    }

    #[test]
    fn permanent_delete_removes_bound_data_metadata_and_record() {
        let (directory, store, layout) = setup("delete");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);

        store
            .delete_permanently(&item, &AtomicBool::new(false))
            .expect("confirmed permanent delete should finish");

        assert!(!source.exists());
        assert!(!item.data_path.exists());
        assert!(!item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn permanent_delete_cancellation_keeps_exact_trash_item_and_no_record() {
        let (directory, store, layout) = setup("delete-cancel");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);

        let error = store
            .delete_permanently(&item, &AtomicBool::new(true))
            .expect_err("cancellation before staging should retain the Trash item");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert!(item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn permanent_delete_refuses_data_substitution_after_listing() {
        let (directory, store, layout) = setup("delete-data-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(&item.data_path, b"changed").unwrap();

        let error = store
            .delete_permanently(&item, &AtomicBool::new(false))
            .expect_err("changed Trash data must not be deleted");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"changed");
        assert!(item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn permanent_delete_refuses_metadata_substitution_after_listing() {
        let (directory, store, layout) = setup("delete-info-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        fs::write(
            &item.info_path,
            b"[Trash Info]\nPath=elsewhere\nDeletionDate=2026-07-29T08:09:10\n",
        )
        .unwrap();

        let error = store
            .delete_permanently(&item, &AtomicBool::new(false))
            .expect_err("changed Trash metadata must not authorize deletion");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert!(item.info_path.exists());
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn permanent_delete_never_follows_a_trashed_symlink() {
        let (directory, store, layout) = setup("delete-symlink");
        let external = directory.0.join("external");
        let source = directory.0.join("folder");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep.txt"), b"keep").unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("remove.txt"), b"remove").unwrap();
        std::os::unix::fs::symlink(&external, source.join("external-link")).unwrap();
        let item = trash_and_list(&store, &layout, &source);

        store
            .delete_permanently(&item, &AtomicBool::new(false))
            .expect("directory deletion should finish");

        assert_eq!(fs::read(external.join("keep.txt")).unwrap(), b"keep");
        assert!(!item.data_path.exists());
        assert!(!item.info_path.exists());
    }

    #[test]
    fn recovery_finishes_delete_renamed_before_stage_persisted() {
        let (directory, store, layout) = setup("delete-rename-crash");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        sync_directory(delete_path.parent().unwrap()).unwrap();

        let recovery = store
            .recover()
            .expect("identity-proven staged delete should recover");

        assert_eq!(
            recovery,
            TrashRecovery {
                finalized: 1,
                pending: 0
            }
        );
        assert!(!delete_path.exists());
        assert!(!item.info_path.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_resumes_a_partially_removed_staged_directory() {
        let (directory, store, layout) = setup("delete-partial-directory");
        let source = directory.0.join("folder");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("first.txt"), b"first").unwrap();
        fs::write(source.join("second.txt"), b"second").unwrap();
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::remove_file(delete_path.join("first.txt")).unwrap();
        sync_directory(&delete_path).unwrap();

        let recovery = store
            .recover()
            .expect("the bound partially deleted directory should resume");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(!delete_path.exists());
        assert!(!item.info_path.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_finishes_delete_after_data_removal_before_stage_persisted() {
        let (directory, store, layout) = setup("delete-data-crash");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::remove_file(&delete_path).unwrap();
        sync_directory(delete_path.parent().unwrap()).unwrap();

        let recovery = store
            .recover()
            .expect("data-removed delete should finish metadata cleanup");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(!item.info_path.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_retains_a_changed_staged_delete_for_review() {
        let (directory, store, layout) = setup("delete-stage-change");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::write(&delete_path, b"changed").unwrap();

        let recovery = store
            .recover()
            .expect("changed staged data should remain pending");

        assert_eq!(recovery.finalized, 0);
        assert_eq!(recovery.pending, 1);
        assert_eq!(fs::read(&delete_path).unwrap(), b"changed");
        assert!(item.info_path.exists());
        assert!(record_path.exists());
    }

    #[test]
    fn reviewed_restore_collision_keeps_both_items_and_clears_only_record() {
        let (directory, store, layout) = setup("review-restore-collision");
        let source = write_source(&directory, OsStr::new("report.txt"), b"trashed");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, _) = prepared_restore_record(&store, &item);
        fs::write(&source, b"existing").unwrap();

        let reviews = store.review_pending().expect("review should be available");

        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].action, TrashRecoveryAction::KeepExistingItems);
        let outcome = store.resolve_review(&reviews[0]).unwrap();
        assert_eq!(outcome, TrashResolutionOutcome::KeptExistingItems);
        assert_eq!(fs::read(&source).unwrap(), b"existing");
        assert_eq!(fs::read(&item.data_path).unwrap(), b"trashed");
        assert!(item.info_path.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn reviewed_changed_delete_stage_returns_remaining_item_to_trash() {
        let (directory, store, layout) = setup("review-delete-stage");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::write(&delete_path, b"changed").unwrap();

        let reviews = store.review_pending().expect("review should be available");

        assert_eq!(reviews.len(), 1);
        assert_eq!(
            reviews[0].action,
            TrashRecoveryAction::ReturnRemainingItem {
                may_be_partial: true
            }
        );
        let outcome = store.resolve_review(&reviews[0]).unwrap();
        assert_eq!(
            outcome,
            TrashResolutionOutcome::ReturnedRemainingItem {
                may_be_partial: true
            }
        );
        assert_eq!(fs::read(&item.data_path).unwrap(), b"changed");
        assert!(!delete_path.exists());
        assert!(item.info_path.exists());
        assert!(!record_path.exists());
        assert_eq!(list_in_layout(&layout).unwrap().len(), 1);
    }

    #[test]
    fn accepted_return_intent_recovers_after_rename_before_record_cleanup() {
        let (directory, store, layout) = setup("review-return-crash");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::write(&delete_path, b"changed").unwrap();
        let review = store.review_pending().unwrap().remove(0);
        let staged = review.delete_snapshot.as_ref().unwrap();
        let info = review.info_snapshot.as_ref().unwrap();
        record.resolution_intent = Some(TrashResolutionIntent::ReturnDeleteStage);
        record.resolution_identity = Some(staged.identity.clone());
        record.resolution_manifest = Some(staged.manifest.clone());
        record.resolution_info_identity = Some(info.identity.clone());
        record.resolution_info_sha256 = Some(info.sha256);
        store.persist(&record_path, &record, false).unwrap();
        rename_noreplace(&delete_path, &item.data_path).unwrap();
        sync_directory(item.data_path.parent().unwrap()).unwrap();

        let recovery = store
            .recover()
            .expect("accepted returned item should finish after restart");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"changed");
        assert!(item.info_path.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn reviewed_conflicting_delete_data_keeps_both_trash_items() {
        let (directory, store, layout) = setup("review-conflicting-delete-data");
        let source = write_source(&directory, OsStr::new("report.txt"), b"staged copy");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::write(&item.data_path, b"visible copy").unwrap();

        let review = store.review_pending().unwrap().remove(0);
        assert_eq!(
            review.action,
            TrashRecoveryAction::PreserveConflictingItems {
                rebuild_metadata: false
            }
        );

        let outcome = store.resolve_review(&review).unwrap();

        assert_eq!(
            outcome,
            TrashResolutionOutcome::PreservedConflictingItems {
                rebuilt_metadata: false
            }
        );
        assert_eq!(fs::read(&item.data_path).unwrap(), b"visible copy");
        assert!(!delete_path.exists());
        assert!(!record_path.exists());
        let items = list_in_layout(&layout).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|item| fs::read(&item.data_path).unwrap() == b"visible copy"));
        assert!(items
            .iter()
            .any(|item| fs::read(&item.data_path).unwrap() == b"staged copy"));
    }

    #[test]
    fn conflicting_delete_data_rebuilds_missing_metadata_for_both_items() {
        let (directory, store, mut layout) = setup("review-conflicting-delete-no-info");
        layout.path_relative_to_topdir = false;
        let source = write_source(&directory, OsStr::new("report.txt"), b"staged copy");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::write(&item.data_path, b"visible copy").unwrap();
        fs::remove_file(&item.info_path).unwrap();

        let review = store.review_pending().unwrap().remove(0);
        assert_eq!(
            review.action,
            TrashRecoveryAction::PreserveConflictingItems {
                rebuild_metadata: true
            }
        );

        let outcome = store.resolve_review(&review).unwrap();

        assert_eq!(
            outcome,
            TrashResolutionOutcome::PreservedConflictingItems {
                rebuilt_metadata: true
            }
        );
        assert!(!delete_path.exists());
        assert!(!record_path.exists());
        assert_eq!(list_in_layout(&layout).unwrap().len(), 2);
    }

    #[test]
    fn accepted_keep_both_intent_recovers_after_hidden_item_publication() {
        let (directory, store, layout) = setup("review-conflicting-delete-crash");
        let source = write_source(&directory, OsStr::new("report.txt"), b"staged copy");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::write(&item.data_path, b"visible copy").unwrap();
        let review = store.review_pending().unwrap().remove(0);
        let primary = review.data_snapshot.as_ref().unwrap();
        let staged = review.delete_snapshot.as_ref().unwrap();
        store
            .begin_conflicting_items_recovery(
                &record_path,
                &mut record,
                primary,
                staged,
                review.info_snapshot.as_ref(),
            )
            .unwrap();
        let recovered_path = record.resolution_data_path().unwrap();
        let recovered_info = record.resolution_info_path().unwrap();
        rename_noreplace(&delete_path, &recovered_path).unwrap();
        sync_directory(recovered_path.parent().unwrap()).unwrap();

        let recovery = store.recover().unwrap();

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"visible copy");
        assert_eq!(fs::read(&recovered_path).unwrap(), b"staged copy");
        assert!(recovered_info.exists());
        assert!(!record_path.exists());
        assert_eq!(list_in_layout(&layout).unwrap().len(), 2);
    }

    #[test]
    fn reviewed_orphan_metadata_removes_no_user_data() {
        let (directory, store, layout) = setup("review-orphan-info");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::remove_file(&delete_path).unwrap();
        fs::write(
            &item.info_path,
            b"[Trash Info]\nPath=changed\nDeletionDate=2026-07-29T08:09:10\n",
        )
        .unwrap();

        let reviews = store.review_pending().expect("review should be available");

        assert_eq!(reviews[0].action, TrashRecoveryAction::RemoveOrphanMetadata);
        let outcome = store.resolve_review(&reviews[0]).unwrap();
        assert_eq!(outcome, TrashResolutionOutcome::RemovedOrphanMetadata);
        assert!(!item.info_path.exists());
        assert!(!record_path.exists());
        assert!(!source.exists());
    }

    #[test]
    fn review_resolution_refuses_a_tree_changed_after_review() {
        let (directory, store, layout) = setup("review-race");
        let source = write_source(&directory, OsStr::new("report.txt"), b"trashed");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, _) = prepared_restore_record(&store, &item);
        fs::write(&source, b"existing").unwrap();
        let reviews = store.review_pending().unwrap();
        fs::write(&item.data_path, b"changed after review").unwrap();

        let error = store
            .resolve_review(&reviews[0])
            .expect_err("changed review must be refused");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(record_path.exists());
        assert_eq!(fs::read(&source).unwrap(), b"existing");
        assert_eq!(fs::read(&item.data_path).unwrap(), b"changed after review");
    }

    #[test]
    fn trash_recovery_review_debug_never_exposes_paths() {
        let (directory, store, layout) = setup("review-debug");
        let source = write_source(&directory, OsStr::new("private-name.txt"), b"trashed");
        let item = trash_and_list(&store, &layout, &source);
        let _ = prepared_restore_record(&store, &item);
        fs::write(&source, b"existing").unwrap();

        let review = store.review_pending().unwrap().remove(0);
        let debug = format!("{review:?}");

        assert!(!debug.contains("private-name"));
        assert!(!debug.contains(directory.0.to_string_lossy().as_ref()));
    }

    #[test]
    fn review_returns_a_hidden_stage_and_rebuilds_missing_metadata() {
        let (directory, store, mut layout) = setup("review-rebuild-hidden");
        layout.path_relative_to_topdir = false;
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        let delete_path = record.delete_path().unwrap();
        rename_noreplace(&item.data_path, &delete_path).unwrap();
        record.stage = TrashStage::DeleteDataStaged;
        store.persist(&record_path, &record, false).unwrap();
        fs::remove_file(&item.info_path).unwrap();

        let reviews = store
            .review_pending()
            .expect("review should remain available");

        assert_eq!(
            reviews[0].action,
            TrashRecoveryAction::ReturnRemainingItemAndRebuildMetadata {
                may_be_partial: false
            }
        );
        let outcome = store.resolve_review(&reviews[0]).unwrap();
        assert_eq!(
            outcome,
            TrashResolutionOutcome::ReturnedRemainingItemAndRebuiltMetadata {
                may_be_partial: false
            }
        );
        assert!(!delete_path.exists());
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert!(item.info_path.exists());
        assert!(!record_path.exists());
        assert_eq!(list_in_layout(&layout).unwrap().len(), 1);
    }

    #[test]
    fn review_rebuilds_missing_metadata_without_changing_trash_data() {
        let (directory, store, mut layout) = setup("review-rebuild-info");
        layout.path_relative_to_topdir = false;
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, _) = prepared_restore_record(&store, &item);
        fs::remove_file(&item.info_path).unwrap();

        let review = store.review_pending().unwrap().remove(0);
        assert_eq!(review.action, TrashRecoveryAction::RebuildMetadata);

        let outcome = store.resolve_review(&review).unwrap();

        assert_eq!(outcome, TrashResolutionOutcome::RebuiltMetadata);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert!(item.info_path.exists());
        assert!(!record_path.exists());
        assert_eq!(list_in_layout(&layout).unwrap().len(), 1);
    }

    #[test]
    fn accepted_metadata_rebuild_recovers_after_create_before_identity_persist() {
        let (directory, store, mut layout) = setup("review-rebuild-crash");
        layout.path_relative_to_topdir = false;
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, _) = prepared_restore_record(&store, &item);
        fs::remove_file(&item.info_path).unwrap();
        let review = store.review_pending().unwrap().remove(0);
        let mut record = review.record;
        let data = review.data_snapshot.unwrap();
        store
            .begin_metadata_rebuild(
                &record_path,
                &mut record,
                TrashResolutionIntent::RebuildMetadata,
                &data,
            )
            .unwrap();
        create_info_file(
            &record.info_path(),
            record.resolution_info_bytes.as_ref().unwrap(),
        )
        .unwrap();

        let recovery = store.recover().unwrap();

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert_eq!(fs::read(&item.data_path).unwrap(), b"important");
        assert!(item.info_path.exists());
        assert!(!record_path.exists());
        assert_eq!(list_in_layout(&layout).unwrap().len(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trash_identity_preserves_non_utf8_path_bytes() {
        let (directory, store, layout) = setup("raw-path");
        let source = write_source(
            &directory,
            &OsString::from_vec(b"resume \xff.txt".to_vec()),
            b"raw",
        );

        store
            .trash_in_layout(&source, &layout, deleted_at(), None)
            .expect("non-UTF-8 file should move to Trash");
        let mut info_name = source.file_name().unwrap().to_os_string();
        info_name.push(".trashinfo");
        let info = fs::read_to_string(layout.root.join("info").join(info_name))
            .expect("metadata should be ASCII");
        let data_path = layout.root.join("files").join(source.file_name().unwrap());

        assert!(info.contains("Path=resume%20%FF.txt\n"));
        assert_eq!(
            data_path.file_name().unwrap().as_bytes(),
            b"resume \xff.txt"
        );
    }

    #[test]
    fn trash_collision_never_overwrites_existing_data() {
        let (directory, store, layout) = setup("collision");
        let source = write_source(&directory, OsStr::new("report.txt"), b"new");
        ensure_trash_layout(&layout).unwrap();
        let existing = layout.root.join("files/report.txt");
        fs::write(&existing, b"existing").unwrap();

        store
            .trash_in_layout(&source, &layout, deleted_at(), None)
            .expect("a unique Trash name should be selected");
        let data_path = layout.root.join("files/report.txt.2");

        assert_eq!(fs::read(existing).unwrap(), b"existing");
        assert_eq!(data_path.file_name().unwrap(), "report.txt.2");
        assert_eq!(fs::read(data_path).unwrap(), b"new");
    }

    #[test]
    fn long_source_name_leaves_room_for_trashinfo_suffix() {
        let (directory, store, layout) = setup("long-name");
        let source_name = "a".repeat(250);
        let source = write_source(&directory, OsStr::new(&source_name), b"long");

        store
            .trash_in_layout(&source, &layout, deleted_at(), None)
            .expect("long valid source name should move to Trash");

        let data_names = fs::read_dir(layout.root.join("files"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        let info_names = fs::read_dir(layout.root.join("info"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(data_names.len(), 1);
        assert_eq!(data_names[0].as_bytes().len(), 245);
        assert_eq!(info_names[0].as_bytes().len(), MAX_FILENAME_BYTES);
        assert!(info_names[0]
            .as_bytes()
            .ends_with(TRASHINFO_SUFFIX.as_bytes()));
    }

    #[test]
    fn cancellation_before_manifest_publication_keeps_source_and_no_record() {
        let (directory, store, layout) = setup("cancel");
        let source = write_source(&directory, OsStr::new("report.txt"), b"important");
        let cancel = AtomicBool::new(true);

        let error = store
            .trash_in_layout(&source, &layout, deleted_at(), Some(&cancel))
            .expect_err("cancelled Trash should stop before publication");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(source.exists());
        assert!(store.read_records().unwrap().is_empty());
        assert!(!layout.root.join("info/report.txt.trashinfo").exists());
        assert!(!layout.root.join("files/report.txt").exists());
    }

    #[test]
    fn recovery_discards_only_a_prepared_record_with_no_side_effects() {
        let (directory, store, layout) = setup("prepared");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (record_path, _, _) = prepared_record(&store, &layout, &source);

        let recovery = store.recover().expect("prepared state should recover");

        assert_eq!(
            recovery,
            TrashRecovery {
                finalized: 1,
                pending: 0
            }
        );
        assert!(source.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_resumes_after_metadata_publication_before_stage_write() {
        let (directory, store, layout) = setup("metadata-crash");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (_, record, info_bytes) = prepared_record(&store, &layout, &source);
        create_info_file(&record.info_path(), &info_bytes)
            .expect("metadata fixture should be published");

        let recovery = store.recover().expect("metadata crash should recover");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(!source.exists());
        assert_eq!(fs::read(record.data_path()).unwrap(), b"draft");
        assert!(store.read_records().unwrap().is_empty());
    }

    #[test]
    fn recovery_finishes_an_identity_proven_interrupted_rename() {
        let (directory, store, layout) = setup("rename-crash");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (record_path, mut record, info_bytes) = prepared_record(&store, &layout, &source);
        record.info_identity = Some(
            create_info_file(&record.info_path(), &info_bytes)
                .expect("metadata fixture should be published"),
        );
        record.stage = TrashStage::InfoPublished;
        store.persist(&record_path, &record, false).unwrap();
        rename_noreplace(&source, &record.data_path()).unwrap();
        sync_directory(record.data_path().parent().unwrap()).unwrap();

        let recovery = store.recover().expect("rename crash should recover");

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(record.data_path().exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn recovery_activates_a_trash_receipt_archived_before_forward_cleanup() {
        let (directory, store, layout) = setup("trash-receipt-crash");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (record_path, mut record, info_bytes) = prepared_record(&store, &layout, &source);
        record.create_undo_receipt = true;
        record.trash_source_parent_identity =
            Some(EntryIdentity::capture(source.parent().unwrap()).unwrap());
        store.persist(&record_path, &record, false).unwrap();
        record.info_identity = Some(create_info_file(&record.info_path(), &info_bytes).unwrap());
        record.stage = TrashStage::InfoPublished;
        store.persist(&record_path, &record, false).unwrap();
        rename_noreplace(&source, &record.data_path()).unwrap();
        sync_directory(source.parent().unwrap()).unwrap();
        sync_directory(record.data_path().parent().unwrap()).unwrap();
        record.stage = TrashStage::DataMoved;
        store.persist(&record_path, &record, false).unwrap();
        store.archive_undo_receipt(&record_path, &record).unwrap();

        assert!(store.undo_store().latest().unwrap().is_none());

        let recovery = store.recover().unwrap();

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(!record_path.exists());
        assert_eq!(
            store.undo_store().latest().unwrap().unwrap().label,
            "Undo Move to Trash “draft.txt”"
        );
    }

    #[test]
    fn recovery_finishes_restore_after_receipt_archive_and_metadata_removal() {
        let (directory, store, layout) = setup("restore-receipt-crash");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_restore_record(&store, &item);
        record.create_undo_receipt = true;
        store.persist(&record_path, &record, false).unwrap();
        rename_noreplace(&record.data_path(), &record.source()).unwrap();
        sync_directory(record.data_path().parent().unwrap()).unwrap();
        sync_directory(record.source().parent().unwrap()).unwrap();
        record.stage = TrashStage::RestoreDataMoved;
        store.persist(&record_path, &record, false).unwrap();
        store.archive_undo_receipt(&record_path, &record).unwrap();
        fs::remove_file(record.info_path()).unwrap();
        sync_directory(record.info_path().parent().unwrap()).unwrap();

        assert_eq!(store.undo_store().count().unwrap(), 2);
        assert_eq!(
            store.undo_store().latest().unwrap().unwrap().label,
            "Undo Move to Trash “draft.txt”"
        );

        let recovery = store.recover().unwrap();

        assert_eq!(recovery.finalized, 1);
        assert_eq!(recovery.pending, 0);
        assert!(!record_path.exists());
        assert_eq!(
            store.undo_store().latest().unwrap().unwrap().label,
            "Undo Restore “draft.txt”"
        );
    }

    #[test]
    fn recovery_retains_changed_metadata_for_review_without_moving_source() {
        let (directory, store, layout) = setup("metadata-substitution");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (record_path, record, info_bytes) = prepared_record(&store, &layout, &source);
        create_info_file(&record.info_path(), &info_bytes).unwrap();
        fs::write(record.info_path(), b"[Trash Info]\nPath=elsewhere\n").unwrap();

        let recovery = store
            .recover()
            .expect("changed metadata should be retained");

        assert_eq!(recovery.finalized, 0);
        assert_eq!(recovery.pending, 1);
        assert!(source.exists());
        assert!(!record.data_path().exists());
        assert!(record_path.exists());
    }

    #[test]
    fn recovery_retains_changed_data_for_review() {
        let (directory, store, layout) = setup("data-substitution");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (record_path, mut record, info_bytes) = prepared_record(&store, &layout, &source);
        record.info_identity = Some(create_info_file(&record.info_path(), &info_bytes).unwrap());
        record.stage = TrashStage::InfoPublished;
        store.persist(&record_path, &record, false).unwrap();
        rename_noreplace(&source, &record.data_path()).unwrap();
        fs::write(record.data_path(), b"changed").unwrap();

        let recovery = store.recover().expect("changed data should be retained");

        assert_eq!(recovery.finalized, 0);
        assert_eq!(recovery.pending, 1);
        assert!(record_path.exists());
    }

    #[test]
    fn concurrent_store_cannot_enter_the_same_recovery_authority() {
        let (directory, store, _) = setup("lock");
        let second = TrashStore::open(directory.0.join("state")).unwrap();
        let _lock = store.lock().expect("first store should own the lock");

        let error = second
            .recover()
            .expect_err("second recovery should not run concurrently");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);

        let error = second
            .execute_latest_undo(
                &crate::file_ops::RealFileSystem,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .expect_err("Undo should share the Trash transaction lock");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn malformed_record_cannot_escape_its_trash_root() {
        let (directory, store, layout) = setup("record-path");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (record_path, mut record, _) = prepared_record(&store, &layout, &source);
        record.data_path_bytes = directory.0.join("outside").as_os_str().as_bytes().to_vec();
        store.persist(&record_path, &record, false).unwrap();

        let error = store
            .recover()
            .expect_err("malformed recovery path should fail closed");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(source.exists());
    }

    #[test]
    fn malformed_delete_record_cannot_stage_outside_trash_data_directory() {
        let (directory, store, layout) = setup("delete-record-path");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let item = trash_and_list(&store, &layout, &source);
        let (record_path, mut record) = prepared_delete_record(&store, &item);
        record.delete_path_bytes =
            Some(directory.0.join("outside").as_os_str().as_bytes().to_vec());
        store.persist(&record_path, &record, false).unwrap();

        let error = store
            .recover()
            .expect_err("malformed delete staging path should fail closed");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(item.data_path.exists());
        assert!(item.info_path.exists());
    }

    #[test]
    fn percent_encoding_preserves_only_uri_safe_path_bytes() {
        assert_eq!(percent_encode_path(b"/a b/%/\xff\n"), "/a%20b/%25/%FF%0A");
        assert_eq!(
            percent_decode_path(b"/a%20b/%25/%FF%0A").unwrap(),
            b"/a b/%/\xff\n"
        );
        assert_eq!(
            percent_decode_path(b"%0").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn version_one_trash_record_without_operation_remains_compatible() {
        let (directory, store, layout) = setup("legacy-record");
        let source = write_source(&directory, OsStr::new("draft.txt"), b"draft");
        let (_, record, _) = prepared_record(&store, &layout, &source);
        let mut value = serde_json::to_value(&record).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("operation");
        object.remove("restore_parent_identity");
        object.remove("create_undo_receipt");
        object.remove("trash_source_parent_identity");
        object.remove("resolution_info_bytes");

        let decoded: TrashRecord = serde_json::from_value(value).unwrap();

        assert_eq!(decoded.operation, TrashOperation::Trash);
        assert!(!decoded.create_undo_receipt);
        decoded.validate(&decoded.id).unwrap();
    }

    #[test]
    fn home_trash_identity_uses_the_absolute_original_path() {
        let directory = TestDirectory::new("home-identity");
        let layout = TrashLayout {
            topdir: directory.0.clone(),
            root: directory.0.join("Trash"),
            path_relative_to_topdir: false,
        };
        let source = directory.0.join("report 1.txt");

        let info = trash_info_bytes(&source, &layout, deleted_at()).unwrap();
        let info = String::from_utf8(info).unwrap();

        assert!(info.contains(&format!(
            "Path={}/report%201.txt\n",
            percent_encode_path(directory.0.as_os_str().as_bytes())
        )));
    }

    #[test]
    fn source_cannot_contain_its_trash_storage() {
        let (directory, store, layout) = setup("recursive-storage");
        let source = directory.0.clone();

        let error = store
            .trash_in_layout(&source, &layout, deleted_at(), None)
            .expect_err("a directory containing Trash storage must be refused");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(directory.0.exists());
        assert!(!layout.root.exists());
    }

    #[test]
    fn mountinfo_parser_decodes_kernel_octal_escapes() {
        assert_eq!(
            decode_mount_field(br"/media/My\040Drive\134Archive").unwrap(),
            b"/media/My Drive\\Archive"
        );
        assert_eq!(
            decode_mount_field(br"/media/bad\x").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn longest_containing_mount_wins() {
        let mounts = vec![
            PathBuf::from("/media/archive"),
            PathBuf::from("/media"),
            PathBuf::from("/"),
        ];

        assert_eq!(
            containing_mount(Path::new("/media/archive/item"), &mounts).unwrap(),
            Path::new("/media/archive")
        );
        assert_eq!(
            containing_mount(Path::new("/home/user/item"), &mounts).unwrap(),
            Path::new("/")
        );
    }

    #[test]
    fn linux_entry_points_are_type_checked_in_host_tests() {
        let _open: fn() -> io::Result<TrashStore> = TrashStore::open_default;
        let _list: fn(&TrashStore) -> io::Result<Vec<TrashedItem>> = TrashStore::list;
        let _trash: fn(&TrashStore, &Path, &AtomicBool) -> io::Result<()> = TrashStore::trash;
        let _restore: fn(&TrashStore, &TrashedItem, &AtomicBool) -> io::Result<PathBuf> =
            TrashStore::restore;
        let _delete: fn(&TrashStore, &TrashedItem, &AtomicBool) -> io::Result<()> =
            TrashStore::delete_permanently;
        let _review: fn(&TrashStore) -> io::Result<Vec<TrashRecoveryReview>> =
            TrashStore::review_pending;
        let _resolve: fn(&TrashStore, &TrashRecoveryReview) -> io::Result<TrashResolutionOutcome> =
            TrashStore::resolve_review;
    }

    #[test]
    fn source_alias_is_bound_to_its_canonical_parent() {
        let directory = TestDirectory::new("source-alias");
        let real = directory.0.join("real");
        let alias = directory.0.join("alias");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        fs::write(real.join("report.txt"), b"report").unwrap();

        let canonical = canonical_source_path(&alias.join("report.txt")).unwrap();

        assert_eq!(canonical, real.join("report.txt"));
    }
}
