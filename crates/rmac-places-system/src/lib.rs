//! Filesystem, portal, and freedesktop Trash adapter for user places.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use sha2::Digest as _;

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
use std::ffi::OsStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolveHome,
    ReadUserDirs,
    InspectPlace,
    InspectTrash,
    OpenDownloads,
    EmptyTrash,
    WatchPlaces,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolveHome => "resolve the home directory",
            Self::ReadUserDirs => "read XDG user directories",
            Self::InspectPlace => "inspect a user place",
            Self::InspectTrash => "inspect Trash",
            Self::OpenDownloads => "open Downloads",
            Self::EmptyTrash => "empty Trash",
            Self::WatchPlaces => "watch user places",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub path: Option<PathBuf>,
    pub error_kind: Option<io::ErrorKind>,
    detail: String,
}

impl Error {
    fn message(operation: Operation, path: Option<&Path>, detail: impl Into<String>) -> Self {
        Self {
            operation,
            path: path.map(Path::to_path_buf),
            error_kind: None,
            detail: detail.into(),
        }
    }

    fn from_io(operation: Operation, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            path: Some(path.to_path_buf()),
            error_kind: Some(error.kind()),
            detail: error.to_string(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}", self.operation)?;
        if let Some(path) = &self.path {
            write!(formatter, " at {}", path.display())?;
        }
        write!(formatter, ": {}", self.detail)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub snapshot: rmac_places::Snapshot,
    pub warnings: Vec<Error>,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct TrashEntryId([u8; 32]);

impl TrashEntryId {
    /// Derive a stable, path-free identity from the platform Trash authority.
    /// Callers never receive the original identifier bytes.
    pub fn from_authority_bytes(bytes: &[u8]) -> Self {
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"rmac-trash-entry-v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }
}

impl fmt::Debug for TrashEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TrashEntryId(<private>)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum WatchEvent {
    Changed,
    Failed { detail: String },
}

impl fmt::Debug for WatchEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed => formatter.write_str("Changed"),
            Self::Failed { .. } => formatter
                .debug_struct("Failed")
                .field("detail", &"<redacted>")
                .finish(),
        }
    }
}

pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
}

pub trait Backend {
    fn home(&self) -> Option<PathBuf>;
    fn config_home(&self) -> Option<PathBuf>;
    fn read_optional(&self, path: &Path) -> io::Result<Option<String>>;
    fn exists(&self, path: &Path) -> io::Result<bool>;
    fn trash_count(&self) -> Result<usize, String>;
    fn trash_entries(&self) -> Result<Vec<TrashEntryId>, String>;
    fn purge_trash(&self, reviewed: &[TrashEntryId]) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl Backend for SystemBackend {
    fn home(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    fn config_home(&self) -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    fn read_optional(&self, path: &Path) -> io::Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn trash_count(&self) -> Result<usize, String> {
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        {
            trash::os_limited::list()
                .map(|items| items.len())
                .map_err(|error| error.to_string())
        }
        #[cfg(not(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )))]
        {
            Err("Trash enumeration is not available on this development platform".into())
        }
    }

    fn trash_entries(&self) -> Result<Vec<TrashEntryId>, String> {
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        {
            trash::os_limited::list()
                .map(|items| items.iter().map(|item| trash_entry_id(&item.id)).collect())
                .map_err(|error| error.to_string())
        }
        #[cfg(not(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )))]
        {
            Err("Trash enumeration is not available on this development platform".into())
        }
    }

    fn purge_trash(&self, reviewed: &[TrashEntryId]) -> Result<(), String> {
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        {
            use std::collections::BTreeMap;

            let current = trash::os_limited::list().map_err(|error| error.to_string())?;
            let mut by_id = BTreeMap::new();
            for item in current {
                if by_id.insert(trash_entry_id(&item.id), item).is_some() {
                    return Err("the Trash authority returned a duplicate item identity".into());
                }
            }
            let mut selected = Vec::with_capacity(reviewed.len());
            for id in reviewed {
                let Some(item) = by_id.remove(id) else {
                    return Err("Trash changed after the deletion review".into());
                };
                selected.push(item);
            }
            trash::os_limited::purge_all(selected).map_err(|error| error.to_string())
        }
        #[cfg(not(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )))]
        {
            let _ = reviewed;
            Err("Empty Trash is not available on this development platform".into())
        }
    }
}

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
fn trash_entry_id(value: &OsStr) -> TrashEntryId {
    use std::os::unix::ffi::OsStrExt as _;

    TrashEntryId::from_authority_bytes(value.as_bytes())
}

pub fn snapshot(backend: &impl Backend) -> Result<Report, Error> {
    let home = backend
        .home()
        .ok_or_else(|| Error::message(Operation::ResolveHome, None, "HOME is not configured"))?;
    if !home.is_absolute() {
        return Err(Error::message(
            Operation::ResolveHome,
            Some(&home),
            "HOME must be absolute",
        ));
    }
    let mut warnings = Vec::new();
    let config_home = backend
        .config_home()
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let user_dirs_path = config_home.join("user-dirs.dirs");
    let user_dirs = match backend.read_optional(&user_dirs_path) {
        Ok(contents) => contents,
        Err(error) => {
            warnings.push(Error::from_io(
                Operation::ReadUserDirs,
                &user_dirs_path,
                error,
            ));
            None
        }
    };
    let downloads = match rmac_places::resolve_downloads(&home, user_dirs.as_deref()) {
        Ok(downloads) => downloads,
        Err(error) => {
            warnings.push(Error::message(
                Operation::ReadUserDirs,
                Some(&user_dirs_path),
                error.to_string(),
            ));
            rmac_places::resolve_downloads(&home, None)
                .expect("an absolute HOME always produces a fallback")
        }
    };
    let home_exists = inspect_place(backend, &home, &mut warnings);
    let downloads_exists = inspect_place(backend, &downloads.path, &mut warnings);
    let trash = match backend.trash_count() {
        Ok(item_count) => rmac_places::TrashSnapshot {
            available: true,
            empty: item_count == 0,
            item_count,
        },
        Err(detail) => {
            warnings.push(Error::message(Operation::InspectTrash, None, detail));
            rmac_places::TrashSnapshot::default()
        }
    };
    Ok(Report {
        snapshot: rmac_places::Snapshot {
            home: rmac_places::Place {
                path: home,
                exists: home_exists,
            },
            downloads: rmac_places::Place {
                path: downloads.path,
                exists: downloads_exists,
            },
            downloads_configured: downloads.configured,
            trash,
        },
        warnings,
    })
}

/// Watch every currently known authority that can change the projected Dock
/// places. The caller resamples a complete snapshot after any hint and then
/// recreates this watcher, so a changed XDG Downloads path or mounted Trash
/// set cannot leave stale watch registrations behind.
pub fn watch(
    report: &Report,
    mut callback: impl FnMut(WatchEvent) + Send + 'static,
) -> Result<Watcher, Error> {
    use notify::Watcher as _;

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let event = match event {
            // Complete resampling reads the watched authorities. Access-only
            // hints must not recursively trigger another resample.
            Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => return,
            Ok(_) => WatchEvent::Changed,
            Err(error) => WatchEvent::Failed {
                detail: error.to_string(),
            },
        };
        callback(event);
    })
    .map_err(|error| Error::message(Operation::WatchPlaces, None, error.to_string()))?;

    let targets = system_watch_targets(report);
    let mut watched = 0usize;
    let mut last_error = None;
    for target in targets {
        match watcher.watch(&target, notify::RecursiveMode::NonRecursive) {
            Ok(()) => watched += 1,
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if watched == 0 {
        return Err(Error::message(
            Operation::WatchPlaces,
            None,
            last_error.unwrap_or_else(|| "no place authority was available to watch".into()),
        ));
    }
    Ok(Watcher { _watcher: watcher })
}

fn system_watch_targets(report: &Report) -> Vec<PathBuf> {
    let home = &report.snapshot.home.path;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"));
    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    let trash_folders = trash::os_limited::trash_folders()
        .map(|folders| folders.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    #[cfg(not(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )))]
    let trash_folders = Vec::new();
    candidate_watch_targets(
        report,
        &config_home,
        &data_home,
        &trash_folders,
        cfg!(target_os = "linux").then_some(Path::new("/proc/self/mounts")),
    )
}

fn candidate_watch_targets(
    report: &Report,
    config_home: &Path,
    data_home: &Path,
    trash_folders: &[PathBuf],
    mounts_file: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = vec![
        config_home.to_path_buf(),
        report.snapshot.home.path.clone(),
        report
            .snapshot
            .downloads
            .path
            .parent()
            .unwrap_or(&report.snapshot.home.path)
            .to_path_buf(),
        data_home.join("Trash"),
    ];
    for folder in trash_folders {
        candidates.push(folder.join("files"));
        candidates.push(folder.join("info"));
    }
    if let Some(mounts_file) = mounts_file {
        candidates.push(mounts_file.to_path_buf());
    }

    candidates
        .into_iter()
        .filter_map(nearest_existing_authority)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn nearest_existing_authority(mut path: PathBuf) -> Option<PathBuf> {
    loop {
        if path.exists() {
            return Some(path);
        }
        if !path.pop() || path == Path::new("/") {
            return None;
        }
    }
}

fn inspect_place(backend: &impl Backend, path: &Path, warnings: &mut Vec<Error>) -> bool {
    match backend.exists(path) {
        Ok(exists) => exists,
        Err(error) => {
            warnings.push(Error::from_io(Operation::InspectPlace, path, error));
            false
        }
    }
}

pub async fn open_downloads(snapshot: &rmac_places::Snapshot) -> Result<(), Error> {
    if !snapshot.downloads.exists {
        return Err(Error::message(
            Operation::OpenDownloads,
            Some(&snapshot.downloads.path),
            "directory is unavailable",
        ));
    }
    rmac_app_launch::reveal_item(snapshot.downloads.path.clone())
        .await
        .map_err(|error| {
            Error::message(
                Operation::OpenDownloads,
                Some(&snapshot.downloads.path),
                error.to_string(),
            )
        })
}

#[derive(Clone, Eq, PartialEq)]
pub struct EmptyTrashReview {
    entries: Vec<TrashEntryId>,
}

impl EmptyTrashReview {
    pub fn item_count(&self) -> usize {
        self.entries.len()
    }
}

impl fmt::Debug for EmptyTrashReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmptyTrashReview")
            .field("item_count", &self.item_count())
            .field("entries", &"<private>")
            .finish()
    }
}

/// Prepare the exact set of Trash identities represented by the accepted
/// snapshot. A changed count refuses review and asks the caller to refresh.
pub fn prepare_empty_trash(
    snapshot: &rmac_places::TrashSnapshot,
    backend: &impl Backend,
) -> Result<Option<EmptyTrashReview>, Error> {
    if !snapshot.available {
        return Err(Error::message(
            Operation::InspectTrash,
            None,
            "Trash is unavailable",
        ));
    }
    if snapshot.empty || snapshot.item_count == 0 {
        return Ok(None);
    }
    let mut entries = backend
        .trash_entries()
        .map_err(|detail| Error::message(Operation::InspectTrash, None, detail))?;
    entries.sort_unstable();
    let before_deduplication = entries.len();
    entries.dedup();
    if entries.len() != before_deduplication || entries.len() != snapshot.item_count {
        return Err(Error::message(
            Operation::InspectTrash,
            None,
            "Trash changed before the deletion review",
        ));
    }
    Ok(Some(EmptyTrashReview { entries }))
}

#[derive(Eq, PartialEq)]
pub struct EmptyTrashConfirmation(EmptyTrashReview);

impl fmt::Debug for EmptyTrashConfirmation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmptyTrashConfirmation")
            .field("item_count", &self.0.item_count())
            .field("entries", &"<private>")
            .finish()
    }
}

pub fn confirm_empty_trash(
    review: EmptyTrashReview,
    confirmed: bool,
) -> Option<EmptyTrashConfirmation> {
    confirmed.then_some(EmptyTrashConfirmation(review))
}

pub fn empty_trash(
    confirmation: EmptyTrashConfirmation,
    backend: &impl Backend,
) -> Result<rmac_places::TrashSnapshot, Error> {
    backend
        .purge_trash(&confirmation.0.entries)
        .map_err(|detail| Error::message(Operation::EmptyTrash, None, detail))?;
    let item_count = backend
        .trash_count()
        .map_err(|detail| Error::message(Operation::InspectTrash, None, detail))?;
    Ok(rmac_places::TrashSnapshot {
        available: true,
        empty: item_count == 0,
        item_count,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct FakeBackend {
        home: Option<PathBuf>,
        config_home: Option<PathBuf>,
        user_dirs: RefCell<io::Result<Option<String>>>,
        existing: Vec<PathBuf>,
        trash_entries: RefCell<Result<Vec<TrashEntryId>, &'static str>>,
        purged: RefCell<Vec<TrashEntryId>>,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self {
                home: Some(PathBuf::from("/home/alex")),
                config_home: None,
                user_dirs: RefCell::new(Ok(None)),
                existing: vec![
                    PathBuf::from("/home/alex"),
                    PathBuf::from("/home/alex/Downloads"),
                ],
                trash_entries: RefCell::new(Ok(Vec::new())),
                purged: RefCell::new(Vec::new()),
            }
        }
    }

    impl Backend for FakeBackend {
        fn home(&self) -> Option<PathBuf> {
            self.home.clone()
        }

        fn config_home(&self) -> Option<PathBuf> {
            self.config_home.clone()
        }

        fn read_optional(&self, _: &Path) -> io::Result<Option<String>> {
            self.user_dirs.replace(Ok(None))
        }

        fn exists(&self, path: &Path) -> io::Result<bool> {
            Ok(self.existing.iter().any(|existing| existing == path))
        }

        fn trash_count(&self) -> Result<usize, String> {
            self.trash_entries
                .borrow()
                .as_ref()
                .map(Vec::len)
                .map_err(|error| (*error).to_owned())
        }

        fn trash_entries(&self) -> Result<Vec<TrashEntryId>, String> {
            self.trash_entries
                .borrow()
                .as_ref()
                .cloned()
                .map_err(|error| (*error).to_owned())
        }

        fn purge_trash(&self, reviewed: &[TrashEntryId]) -> Result<(), String> {
            let mut inventory = self.trash_entries.borrow_mut();
            let entries = inventory.as_mut().map_err(|error| (*error).to_owned())?;
            if reviewed.iter().any(|reviewed| !entries.contains(reviewed)) {
                return Err("Trash changed after the deletion review".into());
            }
            self.purged.borrow_mut().extend_from_slice(reviewed);
            entries.retain(|entry| !reviewed.contains(entry));
            Ok(())
        }
    }

    fn trash_entries(count: u8) -> RefCell<Result<Vec<TrashEntryId>, &'static str>> {
        RefCell::new(Ok((0..count)
            .map(|index| TrashEntryId::from_authority_bytes(&[index]))
            .collect()))
    }

    #[test]
    fn snapshot_uses_configured_downloads_and_complete_trash_count() {
        let backend = FakeBackend {
            user_dirs: RefCell::new(Ok(Some("XDG_DOWNLOAD_DIR=\"$HOME/Transfers\"\n".into()))),
            existing: vec![
                PathBuf::from("/home/alex"),
                PathBuf::from("/home/alex/Transfers"),
            ],
            trash_entries: trash_entries(3),
            ..Default::default()
        };
        let report = snapshot(&backend).expect("snapshot succeeds");
        assert_eq!(
            report.snapshot.downloads.path,
            Path::new("/home/alex/Transfers")
        );
        assert!(report.snapshot.downloads.exists);
        assert!(report.snapshot.downloads_configured);
        assert_eq!(report.snapshot.trash.item_count, 3);
        assert!(!report.snapshot.trash.empty);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn malformed_user_dirs_falls_back_with_a_visible_warning() {
        let backend = FakeBackend {
            user_dirs: RefCell::new(Ok(Some("XDG_DOWNLOAD_DIR=\"relative\"\n".into()))),
            ..Default::default()
        };
        let report = snapshot(&backend).expect("fallback succeeds");
        assert_eq!(
            report.snapshot.downloads.path,
            Path::new("/home/alex/Downloads")
        );
        assert_eq!(report.warnings[0].operation, Operation::ReadUserDirs);
    }

    #[test]
    fn trash_failure_does_not_hide_other_places() {
        let backend = FakeBackend {
            trash_entries: RefCell::new(Err("mount disappeared")),
            ..Default::default()
        };
        let report = snapshot(&backend).expect("places remain available");
        assert!(report.snapshot.downloads.exists);
        assert!(!report.snapshot.trash.available);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.operation == Operation::InspectTrash));
    }

    #[test]
    fn empty_trash_requires_confirmation_and_refreshes_authority() {
        let backend = FakeBackend {
            trash_entries: trash_entries(2),
            ..Default::default()
        };
        let snapshot = rmac_places::TrashSnapshot {
            available: true,
            empty: false,
            item_count: 2,
        };
        let review = prepare_empty_trash(&snapshot, &backend)
            .expect("review prepares")
            .expect("nonempty review");
        assert_eq!(review.item_count(), 2);
        assert!(confirm_empty_trash(review.clone(), false).is_none());
        let confirmation = confirm_empty_trash(review, true).expect("user confirmed");
        let refreshed = empty_trash(confirmation, &backend).expect("purge succeeds");
        assert_eq!(backend.purged.borrow().len(), 2);
        assert!(refreshed.empty);
        assert_eq!(refreshed.item_count, 0);
    }

    #[test]
    fn items_added_after_review_are_never_purged() {
        let backend = FakeBackend {
            trash_entries: trash_entries(2),
            ..Default::default()
        };
        let snapshot = rmac_places::TrashSnapshot {
            available: true,
            empty: false,
            item_count: 2,
        };
        let review = prepare_empty_trash(&snapshot, &backend)
            .expect("review prepares")
            .expect("nonempty review");
        let added = TrashEntryId::from_authority_bytes(b"added after confirmation");
        backend
            .trash_entries
            .borrow_mut()
            .as_mut()
            .expect("inventory")
            .push(added);

        let refreshed = empty_trash(
            confirm_empty_trash(review, true).expect("confirmed"),
            &backend,
        )
        .expect("only reviewed entries purge");
        assert_eq!(backend.purged.borrow().len(), 2);
        assert_eq!(backend.trash_entries().unwrap(), [added]);
        assert_eq!(refreshed.item_count, 1);
        assert!(!refreshed.empty);
    }

    #[test]
    fn stale_or_duplicate_review_authority_fails_closed() {
        let backend = FakeBackend {
            trash_entries: trash_entries(2),
            ..Default::default()
        };
        let snapshot = rmac_places::TrashSnapshot {
            available: true,
            empty: false,
            item_count: 2,
        };
        let review = prepare_empty_trash(&snapshot, &backend)
            .expect("review prepares")
            .expect("nonempty review");
        backend
            .trash_entries
            .borrow_mut()
            .as_mut()
            .expect("inventory")
            .pop();
        assert!(empty_trash(
            confirm_empty_trash(review, true).expect("confirmed"),
            &backend,
        )
        .is_err());
        assert!(backend.purged.borrow().is_empty());

        let duplicate = TrashEntryId::from_authority_bytes(b"same");
        let backend = FakeBackend {
            trash_entries: RefCell::new(Ok(vec![duplicate, duplicate])),
            ..Default::default()
        };
        assert!(prepare_empty_trash(&snapshot, &backend).is_err());
    }

    #[test]
    fn empty_trash_review_debug_redacts_entry_authority() {
        let backend = FakeBackend {
            trash_entries: trash_entries(1),
            ..Default::default()
        };
        let review = prepare_empty_trash(
            &rmac_places::TrashSnapshot {
                available: true,
                empty: false,
                item_count: 1,
            },
            &backend,
        )
        .unwrap()
        .unwrap();
        let debug = format!("{review:?}");
        assert!(debug.contains("<private>"));
        assert!(!debug.contains("TrashEntryId"));
    }

    #[test]
    fn watch_targets_cover_config_place_parents_and_every_known_trash_bin() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rmac-places-watch-{}-{unique}", std::process::id()));
        let home = root.join("home");
        let config = home.join(".config");
        let data = home.join(".local/share");
        let downloads_parent = root.join("media");
        let trash = root.join("mounted/.Trash-1000");
        for path in [&config, &data, &downloads_parent, &trash] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::create_dir_all(trash.join("files")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        let mounts = root.join("mounts");
        std::fs::write(&mounts, []).unwrap();
        let report = Report {
            snapshot: rmac_places::Snapshot {
                home: rmac_places::Place {
                    path: home.clone(),
                    exists: true,
                },
                downloads: rmac_places::Place {
                    path: downloads_parent.join("Downloads"),
                    exists: false,
                },
                downloads_configured: true,
                trash: rmac_places::TrashSnapshot::default(),
            },
            warnings: Vec::new(),
        };

        let targets = candidate_watch_targets(
            &report,
            &config,
            &data,
            std::slice::from_ref(&trash),
            Some(&mounts),
        );
        for expected in [
            config,
            home,
            downloads_parent,
            data,
            trash.join("files"),
            trash.join("info"),
            mounts,
        ] {
            assert!(targets.contains(&expected), "missing {expected:?}");
        }
        assert_eq!(targets.iter().collect::<BTreeSet<_>>().len(), targets.len());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn watch_failure_debug_redacts_backend_diagnostics() {
        let event = WatchEvent::Failed {
            detail: "/home/alex/private mount failed".into(),
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("alex"));
    }
}
