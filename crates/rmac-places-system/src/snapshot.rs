use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::{Backend, Error, Operation, Report, WatchEvent, Watcher};

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

    let (targets, interests) = system_watch_plan(report);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let event = match event {
            // Complete resampling reads the watched authorities. Access-only
            // hints must not recursively trigger another resample.
            Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => return,
            // Home and the config directory are watched only to see a few
            // named entries appear or vanish; ordinary file writes there must
            // not rescan every mount's Trash.
            Ok(event) if !interests.wants_any(&event.paths) => return,
            Ok(_) => WatchEvent::Changed,
            Err(error) => WatchEvent::Failed {
                detail: error.to_string(),
            },
        };
        callback(event);
    })
    .map_err(|error| Error::message(Operation::WatchPlaces, None, error.to_string()))?;

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

fn system_watch_plan(report: &Report) -> (Vec<PathBuf>, Interests) {
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
    let mounts_file = cfg!(target_os = "linux").then_some(Path::new("/proc/self/mounts"));
    (
        candidate_watch_targets(
            report,
            &config_home,
            &data_home,
            &trash_folders,
            mounts_file,
        ),
        candidate_interests(
            report,
            &config_home,
            &data_home,
            &trash_folders,
            mounts_file,
        ),
    )
}

/// The paths whose changes can alter the projected Dock places.
pub(crate) struct Interests {
    /// Only the appearance, removal or replacement of these paths matters.
    existence: Vec<PathBuf>,
    /// Entries created or removed directly inside these paths matter too.
    contents: Vec<PathBuf>,
}

impl Interests {
    /// Events without paths (such as a queue overflow) are always relevant.
    pub(crate) fn wants_any(&self, paths: &[PathBuf]) -> bool {
        paths.is_empty() || paths.iter().any(|path| self.wants(path))
    }

    fn wants(&self, path: &Path) -> bool {
        let names_or_encloses = |target: &PathBuf| target.starts_with(path);
        self.existence.iter().any(names_or_encloses)
            || self.contents.iter().any(names_or_encloses)
            || self
                .contents
                .iter()
                .any(|target| path.parent() == Some(target.as_path()))
    }
}

pub(crate) fn candidate_interests(
    report: &Report,
    config_home: &Path,
    data_home: &Path,
    trash_folders: &[PathBuf],
    mounts_file: Option<&Path>,
) -> Interests {
    let mut contents = vec![data_home.join("Trash/files"), data_home.join("Trash/info")];
    for folder in trash_folders {
        contents.push(folder.join("files"));
        contents.push(folder.join("info"));
    }
    contents.extend(mounts_file.map(Path::to_path_buf));
    Interests {
        existence: vec![
            config_home.join("user-dirs.dirs"),
            report.snapshot.home.path.clone(),
            report.snapshot.downloads.path.clone(),
            data_home.join("Trash"),
        ],
        contents,
    }
}

pub(crate) fn candidate_watch_targets(
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
