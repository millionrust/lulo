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

/// List one directory. Entries that vanish while reading are skipped; any
/// other failure is returned so the caller can say why the folder is empty.
pub fn read_directory(directory: &Path, show_hidden: bool) -> std::io::Result<Vec<Item>> {
    let mut items = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !show_hidden && entry.file_name().to_string_lossy().starts_with('.') {
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
