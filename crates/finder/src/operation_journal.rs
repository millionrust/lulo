use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const JOURNAL_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MoveStage {
    Prepared,
    DestinationComplete,
    SourceRemoved,
    Published,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct EntryIdentity {
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
    fn capture(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    /// Renaming an entry can legitimately update ctime. The durable content
    /// identity must otherwise remain the same across publication.
    fn same_entry_after_rename(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.mode == other.mode
            && self.size == other.size
            && self.modified_seconds == other.modified_seconds
            && self.modified_nanoseconds == other.modified_nanoseconds
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MoveRecord {
    version: u32,
    id: String,
    stage: MoveStage,
    source_path_bytes: Vec<u8>,
    destination_path_bytes: Vec<u8>,
    staging_path_bytes: Vec<u8>,
    source_identity: EntryIdentity,
    destination_identity: Option<EntryIdentity>,
}

impl MoveRecord {
    fn source(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.source_path_bytes.clone()))
    }

    fn destination(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.destination_path_bytes.clone()))
    }

    fn staging_destination(&self) -> PathBuf {
        PathBuf::from(OsString::from_vec(self.staging_path_bytes.clone()))
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
        if self.stage == MoveStage::Prepared && self.destination_identity.is_some() {
            return Err(invalid_data(
                "prepared journal unexpectedly has a destination identity",
            ));
        }
        if self.stage != MoveStage::Prepared && self.destination_identity.is_none() {
            return Err(invalid_data(
                "advanced journal is missing its destination identity",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Journal {
    root: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecoveryReport {
    pub(crate) finalized: usize,
    pub(crate) pending: usize,
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
            let path = self.record_path(&record.id);
            let mut ticket = MoveTicket {
                journal: self.clone(),
                path,
                record,
            };
            if self.recover_ticket(&mut ticket)? {
                report.finalized += 1;
            } else {
                report.pending += 1;
            }
        }
        Ok(report)
    }

    fn recover_ticket(&self, ticket: &mut MoveTicket) -> io::Result<bool> {
        let source = capture_optional(&ticket.record.source())?;
        let staging = capture_optional(&ticket.record.staging_destination())?;
        let destination = capture_optional(&ticket.record.destination())?;
        let source_missing = source.is_none();
        let staging_matches = staging
            .as_ref()
            .zip(ticket.record.destination_identity.as_ref())
            .is_some_and(|(current, recorded)| current == recorded);
        let destination_matches = destination
            .as_ref()
            .zip(ticket.record.destination_identity.as_ref())
            .is_some_and(|(current, recorded)| recorded.same_entry_after_rename(current));

        match ticket.record.stage {
            MoveStage::Prepared => Ok(false),
            MoveStage::DestinationComplete
                if source_missing && staging_matches && destination.is_none() =>
            {
                ticket.mark_source_removed()?;
                self.publish_recovered_ticket(ticket)
            }
            MoveStage::SourceRemoved
                if source_missing && staging_matches && destination.is_none() =>
            {
                self.publish_recovered_ticket(ticket)
            }
            MoveStage::SourceRemoved
                if source_missing && staging.is_none() && destination_matches =>
            {
                ticket.record.destination_identity = destination;
                ticket.record.stage = MoveStage::Published;
                self.persist(&ticket.path, &ticket.record, false)?;
                self.finish_recovered_ticket(ticket)
            }
            MoveStage::Published if source_missing && destination_matches => {
                self.finish_recovered_ticket(ticket)
            }
            _ => Ok(false),
        }
    }

    fn publish_recovered_ticket(&self, ticket: &mut MoveTicket) -> io::Result<bool> {
        match ticket.publish() {
            Ok(()) => self.finish_recovered_ticket(ticket),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn finish_recovered_ticket(&self, ticket: &MoveTicket) -> io::Result<bool> {
        if ticket.record.stage != MoveStage::Published {
            return Err(invalid_data("recovered move did not reach published state"));
        }
        fs::remove_file(&ticket.path)?;
        sync_directory(&self.root)?;
        Ok(true)
    }

    pub(crate) fn prepare_move(&self, source: &Path, destination: &Path) -> io::Result<MoveTicket> {
        if !source.is_absolute() || !destination.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "journaled moves require absolute paths",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let destination_parent = destination.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "journaled move destination has no parent",
            )
        })?;
        let staging = destination_parent.join(format!(".rmac-transfer-{id}"));
        let record = MoveRecord {
            version: JOURNAL_VERSION,
            id: id.clone(),
            stage: MoveStage::Prepared,
            source_path_bytes: source.as_os_str().as_bytes().to_vec(),
            destination_path_bytes: destination.as_os_str().as_bytes().to_vec(),
            staging_path_bytes: staging.as_os_str().as_bytes().to_vec(),
            source_identity: EntryIdentity::capture(source)?,
            destination_identity: None,
        };
        record.validate(&id)?;
        let path = self.record_path(&id);
        self.persist(&path, &record, true)?;
        Ok(MoveTicket {
            journal: self.clone(),
            path,
            record,
        })
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn persist(&self, path: &Path, record: &MoveRecord, create: bool) -> io::Result<()> {
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

    fn read_records(&self) -> io::Result<Vec<MoveRecord>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid_data("journal entry name is not UTF-8"))?;
            if !name.ends_with(".json") {
                return Err(invalid_data("unexpected file in operation journal"));
            }
            paths.push(path);
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
            let record: MoveRecord = serde_json::from_slice(&bytes).map_err(invalid_json)?;
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
            if !(name.starts_with('.') && name.ends_with(".tmp")) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(invalid_data(
                    "abandoned journal temporary is not a regular file",
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

pub(crate) struct MoveTicket {
    journal: Journal,
    path: PathBuf,
    record: MoveRecord,
}

impl MoveTicket {
    pub(crate) fn staging_destination(&self) -> PathBuf {
        self.record.staging_destination()
    }

    pub(crate) fn source_still_matches(&self) -> io::Result<bool> {
        match EntryIdentity::capture(&self.record.source()) {
            Ok(identity) => Ok(identity == self.record.source_identity),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn mark_destination_complete(&mut self) -> io::Result<()> {
        self.record.destination_identity =
            Some(EntryIdentity::capture(&self.record.staging_destination())?);
        self.record.stage = MoveStage::DestinationComplete;
        self.journal.persist(&self.path, &self.record, false)
    }

    pub(crate) fn destination_still_matches(&self) -> io::Result<bool> {
        let path = if self.record.stage == MoveStage::Published {
            self.record.destination()
        } else {
            self.record.staging_destination()
        };
        match EntryIdentity::capture(&path) {
            Ok(identity) => Ok(Some(identity) == self.record.destination_identity),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn mark_source_removed(&mut self) -> io::Result<()> {
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
        self.record.stage = MoveStage::SourceRemoved;
        self.journal.persist(&self.path, &self.record, false)
    }

    pub(crate) fn publish(&mut self) -> io::Result<()> {
        if self.record.stage != MoveStage::SourceRemoved {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "move cannot publish before source removal",
            ));
        }
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
                .ok_or_else(|| invalid_data("move destination has no parent"))?,
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
        self.record.destination_identity = Some(published_identity);
        self.record.stage = MoveStage::Published;
        self.journal.persist(&self.path, &self.record, false)
    }

    pub(crate) fn commit(self) -> io::Result<()> {
        if self.record.stage != MoveStage::Published {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "move journal cannot commit before destination publication",
            ));
        }
        fs::remove_file(self.path)?;
        sync_directory(&self.journal.root)
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
    fn successful_move_lifecycle_removes_the_durable_record() {
        let root = TestDirectory::new("commit");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::write(&source, b"source").unwrap();
        let journal = Journal::open(root.0.join("journal")).unwrap();
        let mut ticket = journal.prepare_move(&source, &destination).unwrap();
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
        let record = MoveRecord {
            version: JOURNAL_VERSION,
            id: id.clone(),
            stage: MoveStage::Prepared,
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
            destination_identity: None,
        };

        let bytes = serde_json::to_vec(&record).unwrap();
        let decoded: MoveRecord = serde_json::from_slice(&bytes).unwrap();

        decoded.validate(&id).unwrap();
        assert_eq!(decoded.source(), source);
        assert_eq!(decoded.destination(), destination);
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
                pending: 0
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
}
