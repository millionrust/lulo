//! ⇧⌘G “Go to Folder”: path resolution and folder completions.

use std::path::{Component, Path, PathBuf};

pub const MAX_SUGGESTIONS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoTo {
    pub folder: PathBuf,
    /// Set when the typed path names a file: the panel opens its folder and
    /// selects it, as the Mac does.
    pub select: Option<PathBuf>,
}

/// Expand `~`, make relative input relative to `cwd`, and fold `.`/`..`.
pub fn expand(input: &str, cwd: &Path, home: &Path) -> Option<PathBuf> {
    let input = input.trim();
    if input.is_empty() || input.contains('\0') || input.len() > 4096 {
        return None;
    }
    let raw = if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else if input.starts_with('/') {
        PathBuf::from(input)
    } else {
        cwd.join(input)
    };
    let mut normal = PathBuf::from("/");
    for component in raw.components() {
        match component {
            Component::Normal(part) => normal.push(part),
            Component::ParentDir => {
                normal.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    Some(normal)
}

pub fn resolve(input: &str, cwd: &Path, home: &Path) -> Option<GoTo> {
    let path = expand(input, cwd, home)?;
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.is_dir() {
        Some(GoTo {
            folder: path,
            select: None,
        })
    } else {
        Some(GoTo {
            folder: path.parent()?.to_path_buf(),
            select: Some(path),
        })
    }
}

/// Folders whose names start with the last typed component (case-insensitive),
/// shown under the field like the Mac's suggestion list.
pub fn suggestions(input: &str, cwd: &Path, home: &Path, show_hidden: bool) -> Vec<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let (parent, prefix) = if trimmed.ends_with('/') || trimmed == "~" {
        match expand(trimmed, cwd, home) {
            Some(parent) => (parent, String::new()),
            None => return Vec::new(),
        }
    } else {
        match expand(trimmed, cwd, home) {
            Some(path) => {
                let prefix = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                match path.parent() {
                    Some(parent) => (parent.to_path_buf(), prefix),
                    None => return Vec::new(),
                }
            }
            None => return Vec::new(),
        }
    };
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            (show_hidden || !name.starts_with('.') || prefix.starts_with('.'))
                && name.starts_with(&prefix)
        })
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .take(256)
        .collect();
    matches.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    matches.truncate(MAX_SUGGESTIONS);
    matches
}

/// How a suggestion reads in the list: `~/Documents` under home, else absolute.
pub fn display(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
