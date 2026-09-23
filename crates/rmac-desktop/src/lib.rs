//! Bounded, framework-neutral Desktop directory projection, plus the pure
//! parts of the macOS 26 desktop: the icon grid (`grid`), Stacks
//! (`stacks`), widgets (`widgets`) and the saved desktop state
//! (`settings`).

pub mod grid;
pub mod settings;
pub mod stacks;
pub mod widgets;

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const MAX_DESKTOP_ITEMS: usize = 512;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortOrder {
    #[default]
    Name,
    Kind,
    DateModified,
    Size,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ItemKind {
    Directory,
    File,
    SymbolicLink,
    Other,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub kind: ItemKind,
    pub size_bytes: u64,
    pub modified_millis: u128,
}

impl fmt::Debug for Item {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Item")
            .field("path", &"<private>")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("size_bytes", &self.size_bytes)
            .field("modified_millis", &self.modified_millis)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Snapshot {
    pub directory: PathBuf,
    pub items: Vec<Item>,
    pub truncated: bool,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("directory", &"<private>")
            .field("items", &self.items)
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[derive(Debug)]
pub enum Error {
    MissingHome,
    InvalidHome,
    InvalidUserDirectory,
    Io(io::ErrorKind),
    Watch,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingHome => "the home directory is unavailable",
            Self::InvalidHome => "the home directory is invalid",
            Self::InvalidUserDirectory => "the configured Desktop directory is invalid",
            Self::Io(_) => "the Desktop directory could not be read",
            Self::Watch => "the Desktop directory could not be watched",
        })
    }
}

impl std::error::Error for Error {}

pub struct Watcher {
    _inner: notify::RecommendedWatcher,
}

pub fn directory_from_environment() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(Error::MissingHome)?;
    if !home.is_absolute() {
        return Err(Error::InvalidHome);
    }
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let contents = match fs::read_to_string(config_home.join("user-dirs.dirs")) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(Error::Io(error.kind())),
    };
    resolve_directory(&home, contents.as_deref())
}

pub fn resolve_directory(home: &Path, user_dirs: Option<&str>) -> Result<PathBuf, Error> {
    if !home.is_absolute() {
        return Err(Error::InvalidHome);
    }
    let Some(raw) = user_dirs.and_then(|contents| {
        contents.lines().find_map(|line| {
            let line = line.trim();
            (!line.starts_with('#'))
                .then(|| line.strip_prefix("XDG_DESKTOP_DIR="))
                .flatten()
        })
    }) else {
        return Ok(home.join("Desktop"));
    };
    let value = parse_quoted(raw)?;
    let path = if let Some(suffix) = value.strip_prefix("$HOME") {
        append_home(home, suffix)?
    } else if let Some(suffix) = value.strip_prefix("${HOME}") {
        append_home(home, suffix)?
    } else {
        if value.contains('$') || value.contains('`') {
            return Err(Error::InvalidUserDirectory);
        }
        PathBuf::from(value)
    };
    path.is_absolute()
        .then_some(path)
        .ok_or(Error::InvalidUserDirectory)
}

pub fn scan(directory: &Path, sort: SortOrder) -> Result<Snapshot, Error> {
    if !directory.is_absolute() {
        return Err(Error::InvalidUserDirectory);
    }
    let mut items = Vec::new();
    let entries = fs::read_dir(directory).map_err(|error| Error::Io(error.kind()))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::Io(error.kind())),
        };
        let name = entry.file_name();
        if hidden(&name) {
            continue;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::Io(error.kind())),
        };
        let kind = if metadata.file_type().is_symlink() {
            ItemKind::SymbolicLink
        } else if metadata.is_dir() {
            ItemKind::Directory
        } else if metadata.is_file() {
            ItemKind::File
        } else {
            ItemKind::Other
        };
        items.push(Item {
            path: entry.path(),
            name: name.to_string_lossy().into_owned(),
            kind,
            size_bytes: metadata.len(),
            modified_millis: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_millis()),
        });
        if items.len() > MAX_DESKTOP_ITEMS {
            break;
        }
    }
    let truncated = items.len() > MAX_DESKTOP_ITEMS;
    items.truncate(MAX_DESKTOP_ITEMS);
    sort_items(&mut items, sort);
    Ok(Snapshot {
        directory: directory.to_path_buf(),
        items,
        truncated,
    })
}

pub fn create_folder(directory: &Path) -> Result<PathBuf, Error> {
    if !directory.is_absolute() {
        return Err(Error::InvalidUserDirectory);
    }
    for index in 0..10_000u32 {
        let name = match index {
            0 => "untitled folder".to_owned(),
            _ => format!("untitled folder {index}"),
        };
        let path = directory.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::Io(error.kind())),
        }
    }
    Err(Error::Io(io::ErrorKind::AlreadyExists))
}

pub fn watch(
    directory: &Path,
    mut changed: impl FnMut() + Send + 'static,
) -> Result<Watcher, Error> {
    use notify::Watcher as _;

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| !matches!(event.kind, notify::EventKind::Access(_))) {
            changed();
        }
    })
    .map_err(|_| Error::Watch)?;
    watcher
        .watch(directory, notify::RecursiveMode::NonRecursive)
        .map_err(|_| Error::Watch)?;
    Ok(Watcher { _inner: watcher })
}

/// Watches one file by name in `directory` (which must exist), calling
/// `changed` whenever it is created, written or removed.
pub fn watch_file(
    directory: &Path,
    file_name: &'static str,
    mut changed: impl FnMut() + Send + 'static,
) -> Result<Watcher, Error> {
    use notify::Watcher as _;

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            return;
        };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event
            .paths
            .iter()
            .any(|path| path.file_name() == Some(OsStr::new(file_name)))
        {
            changed();
        }
    })
    .map_err(|_| Error::Watch)?;
    watcher
        .watch(directory, notify::RecursiveMode::NonRecursive)
        .map_err(|_| Error::Watch)?;
    Ok(Watcher { _inner: watcher })
}

/// A unique "name copy" path for Duplicate, as Finder names copies.
pub fn duplicate_path(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_owned(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    (1..10_000u32)
        .map(|index| match index {
            1 => parent.join(format!("{stem} copy{extension}")),
            _ => parent.join(format!("{stem} copy {index}{extension}")),
        })
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
}

/// Copies a regular file to [`duplicate_path`]; folders and links are not
/// duplicated.
pub fn duplicate_file(path: &Path) -> Result<PathBuf, Error> {
    let metadata = fs::symlink_metadata(path).map_err(|error| Error::Io(error.kind()))?;
    if !metadata.is_file() {
        return Err(Error::Io(io::ErrorKind::Unsupported));
    }
    let target = duplicate_path(path).ok_or(Error::Io(io::ErrorKind::AlreadyExists))?;
    let mut source = fs::File::open(path).map_err(|error| Error::Io(error.kind()))?;
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|error| Error::Io(error.kind()))?;
    io::copy(&mut source, &mut destination).map_err(|error| Error::Io(error.kind()))?;
    Ok(target)
}

fn sort_items(items: &mut [Item], sort: SortOrder) {
    items.sort_by(|left, right| {
        let order = match sort {
            SortOrder::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            SortOrder::Kind => stacks::kind_label(left).cmp(stacks::kind_label(right)),
            SortOrder::DateModified => left.modified_millis.cmp(&right.modified_millis).reverse(),
            SortOrder::Size => left.size_bytes.cmp(&right.size_bytes).reverse(),
        };
        order.then_with(|| left.name.cmp(&right.name))
    });
}

fn hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

fn append_home(home: &Path, suffix: &str) -> Result<PathBuf, Error> {
    if suffix.contains('$')
        || suffix.contains('`')
        || (!suffix.is_empty() && !suffix.starts_with('/'))
    {
        return Err(Error::InvalidUserDirectory);
    }
    Ok(suffix
        .strip_prefix('/')
        .filter(|suffix| !suffix.is_empty())
        .map_or_else(|| home.to_path_buf(), |suffix| home.join(suffix)))
}

fn parse_quoted(raw: &str) -> Result<String, Error> {
    let raw = raw.trim();
    if raw.len() < 2 || !raw.starts_with('"') || !raw.ends_with('"') {
        return Err(Error::InvalidUserDirectory);
    }
    let mut result = String::new();
    let mut characters = raw[1..raw.len() - 1].chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => result.push('\\'),
            Some('"') => result.push('"'),
            Some('$') => result.push('$'),
            _ => return Err(Error::InvalidUserDirectory),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn temporary(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rmac-desktop-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn desktop_directory_resolution_is_shell_free_and_bounded() {
        let home = Path::new("/home/alex");
        assert_eq!(resolve_directory(home, None).unwrap(), home.join("Desktop"));
        assert_eq!(
            resolve_directory(home, Some("XDG_DESKTOP_DIR=\"$HOME/Desk\"\n")).unwrap(),
            home.join("Desk")
        );
        for invalid in [
            "XDG_DESKTOP_DIR=relative\n",
            "XDG_DESKTOP_DIR=\"relative\"\n",
            "XDG_DESKTOP_DIR=\"$OTHER/Desk\"\n",
            "XDG_DESKTOP_DIR=\"`touch /tmp/no`\"\n",
        ] {
            assert!(resolve_directory(home, Some(invalid)).is_err());
        }
    }

    #[test]
    fn scan_hides_dotfiles_sorts_and_never_follows_symlinks() {
        let root = temporary("scan");
        fs::write(root.join("Beta.txt"), b"1234").unwrap();
        fs::write(root.join("alpha.txt"), b"12").unwrap();
        fs::write(root.join(".secret"), b"private").unwrap();
        fs::create_dir(root.join("Folder")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("Beta.txt"), root.join("Link")).unwrap();
        let snapshot = scan(&root, SortOrder::Name).unwrap();
        assert_eq!(
            snapshot
                .items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha.txt", "Beta.txt", "Folder", "Link"]
        );
        #[cfg(unix)]
        assert_eq!(snapshot.items[3].kind, ItemKind::SymbolicLink);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicates_are_named_like_finder_copies() {
        let root = temporary("duplicate");
        fs::write(root.join("Report.pdf"), b"1").unwrap();
        let first = duplicate_file(&root.join("Report.pdf")).unwrap();
        assert_eq!(first, root.join("Report copy.pdf"));
        let second = duplicate_file(&root.join("Report.pdf")).unwrap();
        assert_eq!(second, root.join("Report copy 2.pdf"));
        fs::create_dir(root.join("Folder")).unwrap();
        assert!(duplicate_file(&root.join("Folder")).is_err());
        assert_eq!(
            duplicate_path(&root.join("notes")).unwrap(),
            root.join("notes copy")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_folder_uses_finder_style_unique_names() {
        let root = temporary("folder");
        assert_eq!(create_folder(&root).unwrap(), root.join("untitled folder"));
        assert_eq!(
            create_folder(&root).unwrap(),
            root.join("untitled folder 1")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
