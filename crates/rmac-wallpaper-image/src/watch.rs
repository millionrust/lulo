use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use crate::cache::failure;
use crate::{Error, ErrorKind};

pub enum FileWatchEvent {
    Changed,
    Failed { detail: String },
}

impl fmt::Debug for FileWatchEvent {
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

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Watch exact selected files and their existing symlink targets. Parent
/// directories are non-recursive so unrelated tree activity is ignored.
pub fn watch_files(
    paths: &[std::path::PathBuf],
    sender: async_channel::Sender<FileWatchEvent>,
) -> Result<Option<FileWatcher>, Error> {
    use notify::Watcher as _;

    if paths.is_empty() {
        return Ok(None);
    }
    let mut targets = BTreeSet::new();
    for path in paths {
        targets.insert(path.clone());
        if let Ok(canonical) = path.canonicalize() {
            targets.insert(canonical);
        }
    }
    let callback_targets = targets.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let event = match result {
            Ok(event)
                if !matches!(event.kind, notify::EventKind::Access(_))
                    && event.paths.iter().any(|path| {
                        callback_targets.contains(path)
                            || callback_targets
                                .iter()
                                .any(|target| target.starts_with(path))
                    }) =>
            {
                Some(FileWatchEvent::Changed)
            }
            Ok(_) => None,
            Err(error) => Some(FileWatchEvent::Failed {
                detail: error.to_string(),
            }),
        };
        if let Some(event) = event {
            let _ = sender.try_send(event);
        }
    })
    .map_err(|error| failure(ErrorKind::Watch, error.to_string()))?;
    let parents: BTreeSet<_> = targets
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    for parent in parents {
        watcher
            .watch(&parent, notify::RecursiveMode::NonRecursive)
            .map_err(|error| failure(ErrorKind::Watch, error.to_string()))?;
    }
    Ok(Some(FileWatcher { _watcher: watcher }))
}
