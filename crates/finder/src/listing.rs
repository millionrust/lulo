//! Framework-neutral directory listing shared by Files and the rmac Open/Save
//! panel, so both describe items with the same kinds, sizes, and dates.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, Timelike};

/// One directory item as both Files and the file chooser present it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub mtime: SystemTime,
    pub kind: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    Kind,
    Date,
    Size,
}

impl Item {
    /// Describe `path` without following a final symlink. A symlink to a
    /// directory is still listed as a folder so it can be browsed into.
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        let link = std::fs::symlink_metadata(path).ok()?;
        let metadata = if link.file_type().is_symlink() {
            std::fs::metadata(path).unwrap_or(link)
        } else {
            link
        };
        let is_dir = metadata.is_dir();
        Some(Self {
            kind: kind_of(path, is_dir),
            name,
            path: path.to_path_buf(),
            is_dir,
            size_bytes: if is_dir { 0 } else { metadata.len() },
            mtime: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        })
    }

    pub fn size_label(&self) -> String {
        if self.is_dir {
            "--".to_owned()
        } else {
            human_size(self.size_bytes)
        }
    }

    pub fn date_label(&self) -> String {
        date_label(self.mtime)
    }

    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// `FILE_ATTRIBUTE_HIDDEN`: Explorer hides the item unless "Show hidden
/// files" is on.
pub const ATTRIBUTE_HIDDEN: u32 = 0x2;
/// `FILE_ATTRIBUTE_SYSTEM`: with [`ATTRIBUTE_HIDDEN`] it marks a protected
/// operating-system file (NTUSER.DAT, desktop.ini), which Explorer hides
/// even then unless "Hide protected operating system files" is off.
pub const ATTRIBUTE_SYSTEM: u32 = 0x4;

/// Whether a folder lists an entry, as Explorer decides on Windows and the
/// Mac (a leading dot) everywhere: with hidden items off, dot-names and
/// hidden items stay out; with them on (⇧⌘., or Explorer's own setting),
/// protected system files still stay out unless `show_protected`.
pub fn is_listed(name: &str, attributes: u32, show_hidden: bool, show_protected: bool) -> bool {
    let hidden = attributes & ATTRIBUTE_HIDDEN != 0;
    let protected = hidden && attributes & ATTRIBUTE_SYSTEM != 0;
    if !show_hidden {
        return !name.starts_with('.') && !hidden;
    }
    !protected || show_protected
}

/// The entry's Windows file attributes (0 elsewhere), from the directory
/// read itself: no extra system call.
pub fn entry_attributes(entry: &std::fs::DirEntry) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        entry
            .metadata()
            .map(|metadata| metadata.file_attributes())
            .unwrap_or(0)
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        0
    }
}

/// Explorer's own choices, read once: whether it shows hidden items, and
/// whether it shows protected operating-system files
/// (`HKCU\…\Explorer\Advanced` `Hidden` = 1, `ShowSuperHidden` = 1).
/// Elsewhere hidden items start hidden and nothing is protected.
pub fn explorer_hidden_settings() -> (bool, bool) {
    static SETTINGS: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    *SETTINGS.get_or_init(|| {
        #[cfg(windows)]
        {
            (
                explorer_advanced("Hidden") == Some(1),
                explorer_advanced("ShowSuperHidden") == Some(1),
            )
        }
        #[cfg(not(windows))]
        {
            (false, true)
        }
    })
}

#[cfg(windows)]
fn explorer_advanced(name: &str) -> Option<u32> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: a DWORD-sized buffer with its size; the key and value names
    // live for the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced"),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}

/// List one directory. Entries that vanish while reading are skipped; any
/// other failure is returned so the caller can say why the folder is empty.
pub fn read_directory(directory: &Path, show_hidden: bool) -> std::io::Result<Vec<Item>> {
    let (_, show_protected) = explorer_hidden_settings();
    let mut items = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !is_listed(
            &entry.file_name().to_string_lossy(),
            entry_attributes(&entry),
            show_hidden,
            show_protected,
        ) {
            continue;
        }
        if let Some(item) = Item::from_path(&entry.path()) {
            items.push(item);
        }
    }
    Ok(items)
}

/// Sort like Files: the chosen key, ties broken by case-insensitive name.
pub fn sort_items(items: &mut [Item], key: SortKey, ascending: bool) {
    items.sort_by(|a, b| {
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        let order = match key {
            SortKey::Name => by_name(),
            SortKey::Kind => a
                .kind
                .to_lowercase()
                .cmp(&b.kind.to_lowercase())
                .then_with(by_name),
            SortKey::Date => a.mtime.cmp(&b.mtime).then_with(by_name),
            SortKey::Size => a.size_bytes.cmp(&b.size_bytes).then_with(by_name),
        };
        if ascending || order == Ordering::Equal {
            order
        } else {
            order.reverse()
        }
    });
}

pub fn human_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K * K {
        format!("{:.2} GB", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.1} MB", b / (K * K))
    } else if b >= K {
        format!("{:.0} KB", b / K)
    } else {
        format!("{bytes} bytes")
    }
}

pub fn kind_of(path: &Path, is_dir: bool) -> String {
    if is_dir {
        return "Folder".to_string();
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "Rust Source".into(),
        "toml" => "TOML Document".into(),
        "md" => "Markdown Document".into(),
        "txt" => "Plain Text Document".into(),
        "json" => "JSON document".into(),
        "lock" => "Document".into(),
        "png" => "PNG image".into(),
        "jpg" | "jpeg" => "JPEG image".into(),
        "gif" => "GIF image".into(),
        "webp" => "WebP image".into(),
        "pdf" => "PDF document".into(),
        "zip" => "ZIP archive".into(),
        "gz" | "tar" => "Archive".into(),
        "app" => "Application".into(),
        "" => "Document".into(),
        other => format!("{} document", other.to_uppercase()),
    }
}

pub fn date_label(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    let now = Local::now();
    let (h12, ap) = {
        let h = dt.hour();
        if h == 0 {
            (12, "AM")
        } else if h < 12 {
            (h, "AM")
        } else if h == 12 {
            (12, "PM")
        } else {
            (h - 12, "PM")
        }
    };
    let time = format!("{}:{:02} {}", h12, dt.minute(), ap);
    let days = now
        .date_naive()
        .signed_duration_since(dt.date_naive())
        .num_days();
    if days == 0 {
        format!("Today at {time}")
    } else if days == 1 {
        format!("Yesterday at {time}")
    } else {
        format!("{} {} {} at {time}", dt.day(), dt.format("%b"), dt.year())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_protected_items_follow_explorer() {
        let hidden = ATTRIBUTE_HIDDEN;
        let protected = ATTRIBUTE_HIDDEN | ATTRIBUTE_SYSTEM;
        // Hidden items off (Explorer's default): only plain items show.
        assert!(is_listed("report.txt", 0, false, false));
        assert!(!is_listed(".profile", 0, false, false));
        assert!(!is_listed("AppData", hidden, false, false));
        assert!(!is_listed("NTUSER.DAT", protected, false, true));
        // A system-only item is not hidden in Explorer either.
        assert!(is_listed("pagefile-like", ATTRIBUTE_SYSTEM, false, false));
        // Hidden items on (⇧⌘. or Explorer's "Show hidden files").
        assert!(is_listed(".profile", 0, true, false));
        assert!(is_listed("AppData", hidden, true, false));
        assert!(!is_listed("desktop.ini", protected, true, false));
        assert!(is_listed("desktop.ini", protected, true, true));
    }

    #[test]
    fn listing_hides_dot_names_and_shows_them_on_request() {
        let root = std::env::temp_dir().join(format!(
            "rmac-listing-hidden-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("plain.txt"), b"x").unwrap();
        std::fs::write(root.join(".dot"), b"x").unwrap();
        let names = |show| {
            let mut names = read_directory(&root, show)
                .unwrap()
                .into_iter()
                .map(|item| item.name)
                .collect::<Vec<_>>();
            names.sort();
            names
        };
        assert_eq!(names(false), ["plain.txt"]);
        assert_eq!(names(true), [".dot", "plain.txt"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sizes_and_kinds_match_files() {
        assert_eq!(human_size(12), "12 bytes");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(kind_of(Path::new("a/b.PNG"), false), "PNG image");
        assert_eq!(kind_of(Path::new("a/b"), true), "Folder");
        assert_eq!(kind_of(Path::new("a/b.xyz"), false), "XYZ document");
    }

    #[test]
    fn listing_hides_dotfiles_and_sorts_by_name() {
        let root = std::env::temp_dir().join(format!("rmac-listing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Beta")).unwrap();
        std::fs::write(root.join("alpha.txt"), b"x").unwrap();
        std::fs::write(root.join(".hidden"), b"x").unwrap();
        let mut items = read_directory(&root, false).unwrap();
        sort_items(&mut items, SortKey::Name, true);
        let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["alpha.txt", "Beta"]);
        assert!(items[1].is_dir);
        assert_eq!(read_directory(&root, true).unwrap().len(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
