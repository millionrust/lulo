//! Archive Utility ▸ Settings… (⌘,): where expanded files and new archives
//! go, what happens to the original afterwards, whether the result is shown
//! in Files, and whether an archive found inside an archive is expanded
//! too. Saved in `~/.config/lulo/archive-utility.json`.
//!
//! The Mac's "Use archive format:" pop-up (Compressed archive (.cpgz),
//! Regular archive (.cpio), ZIP archive) is left out: Lulo writes ZIP only
//! (`rmac_archive::compress`), so the pop-up would offer one choice.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "lulo/archive-utility.json";
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
/// How deep "Keep expanding if possible" follows archives inside archives.
const MAX_NESTED_EXPANSIONS: usize = 4;

/// What happens to the archive (After expanding:) or to the archived items
/// (After archiving:) once the job succeeds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub enum Afterwards {
    #[default]
    Leave,
    MoveToBin,
    Delete,
}

impl Afterwards {
    pub const ALL: [Self; 3] = [Self::Leave, Self::MoveToBin, Self::Delete];

    pub fn expanding_label(self) -> &'static str {
        match self {
            Self::Leave => "leave archive alone",
            Self::MoveToBin => "move archive to Bin",
            Self::Delete => "delete archive",
        }
    }

    pub fn archiving_label(self) -> &'static str {
        match self {
            Self::Leave => "leave files alone",
            Self::MoveToBin => "move files to Bin",
            Self::Delete => "delete files",
        }
    }

    /// Apply this choice to `path` (the archive, or one archived item).
    pub fn apply(self, path: &Path) -> io::Result<()> {
        match self {
            Self::Leave => Ok(()),
            Self::MoveToBin => {
                trash::delete(path).map_err(|error| io::Error::other(error.to_string()))
            }
            Self::Delete => {
                if fs::symlink_metadata(path)?.is_dir() {
                    fs::remove_dir_all(path)
                } else {
                    fs::remove_file(path)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    /// Save expanded files: `None` is "into same folder as archive".
    pub expand_into: Option<PathBuf>,
    pub after_expanding: Afterwards,
    pub reveal_expanded: bool,
    pub keep_expanding: bool,
    /// Save archives: `None` is "into same folder as originals".
    pub archive_into: Option<PathBuf>,
    pub after_archiving: Afterwards,
    pub reveal_archive: bool,
}

impl Default for Settings {
    /// The Mac's defaults: beside the original, leave it, don't reveal,
    /// keep expanding.
    fn default() -> Self {
        Self {
            expand_into: None,
            after_expanding: Afterwards::Leave,
            reveal_expanded: false,
            keep_expanding: true,
            archive_into: None,
            after_archiving: Afterwards::Leave,
            reveal_archive: false,
        }
    }
}

impl Settings {
    /// A chosen folder that has since gone away falls back to "same
    /// folder", as on the Mac.
    pub fn expand_destination(&self, archive: &Path) -> PathBuf {
        self.expand_into
            .clone()
            .filter(|folder| folder.is_dir())
            .unwrap_or_else(|| parent_of(archive))
    }

    pub fn archive_destination(&self, first_item: &Path) -> PathBuf {
        self.archive_into
            .clone()
            .filter(|folder| folder.is_dir())
            .unwrap_or_else(|| parent_of(first_item))
    }
}

fn parent_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

pub fn load_from(path: &Path) -> Settings {
    rmac_storage::read_bounded_no_follow(path, MAX_SETTINGS_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    rmac_storage::atomic_write_private(path, &bytes)
}

pub fn load() -> Settings {
    config_root()
        .map(|root| load_from(&root.join(SETTINGS_FILE)))
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let root = config_root().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    save_to(&root.join(SETTINGS_FILE), settings)
}

/// Expand `archive` as `settings` ask: into the chosen folder, then (Keep
/// expanding if possible) expand a single archive that came out of it in
/// turn, removing that intermediate file. Returns what is left to show.
pub fn expand(
    archive: &Path,
    settings: &Settings,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(rmac_archive::Progress),
) -> Result<PathBuf, rmac_archive::Error> {
    let mut placed = rmac_archive::expand_into(
        archive,
        &settings.expand_destination(archive),
        cancel,
        progress,
    )?;
    if settings.keep_expanding {
        for _ in 0..MAX_NESTED_EXPANSIONS {
            if !placed.is_file() || rmac_archive::format_of(&placed).is_none() {
                break;
            }
            match rmac_archive::expand(&placed, cancel, progress) {
                Ok(inner) => {
                    let _ = fs::remove_file(&placed);
                    placed = inner;
                }
                // Named like an archive but not one: keep it as it came out.
                Err(rmac_archive::Error::Unsupported | rmac_archive::Error::Damaged) => break,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rmac-archive-utility-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn settings_round_trip_and_default_to_the_macs_choices() {
        let root = scratch("settings");
        let path = root.join("lulo/archive-utility.json");
        assert_eq!(load_from(&path), Settings::default());
        assert!(Settings::default().keep_expanding);
        let settings = Settings {
            expand_into: Some(root.clone()),
            after_expanding: Afterwards::MoveToBin,
            reveal_expanded: true,
            keep_expanding: false,
            archive_into: None,
            after_archiving: Afterwards::Delete,
            reveal_archive: true,
        };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path), settings);
        fs::write(&path, b"not json").unwrap();
        assert_eq!(load_from(&path), Settings::default());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_chosen_folder_falls_back_to_the_originals_folder() {
        let root = scratch("destination");
        let archive = root.join("a.zip");
        let mut settings = Settings {
            expand_into: Some(root.join("gone")),
            ..Settings::default()
        };
        assert_eq!(settings.expand_destination(&archive), root);
        let chosen = root.join("Chosen");
        fs::create_dir(&chosen).unwrap();
        settings.expand_into = Some(chosen.clone());
        assert_eq!(settings.expand_destination(&archive), chosen);
        assert_eq!(settings.archive_destination(&archive), root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_afterwards_removes_files_and_folders() {
        let root = scratch("afterwards");
        let file = root.join("a.txt");
        fs::write(&file, "a").unwrap();
        let folder = root.join("Folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("b.txt"), "b").unwrap();
        Afterwards::Leave.apply(&file).unwrap();
        assert!(file.exists());
        Afterwards::Delete.apply(&file).unwrap();
        Afterwards::Delete.apply(&folder).unwrap();
        assert!(!file.exists() && !folder.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keep_expanding_opens_an_archive_inside_an_archive() {
        let root = scratch("nested");
        fs::write(root.join("notes.txt"), "one").unwrap();
        let cancel = AtomicBool::new(false);
        let inner =
            rmac_archive::compress(&[root.join("notes.txt")], &cancel, &mut |_| {}).unwrap();
        fs::remove_file(root.join("notes.txt")).unwrap();
        let holder = root.join("Holder");
        fs::create_dir(&holder).unwrap();
        let moved = holder.join("notes.txt.zip");
        fs::rename(&inner, &moved).unwrap();
        let outer =
            rmac_archive::compress(std::slice::from_ref(&moved), &cancel, &mut |_| {}).unwrap();
        fs::remove_file(&moved).unwrap();

        let shallow = Settings {
            keep_expanding: false,
            ..Settings::default()
        };
        let once = expand(&outer, &shallow, &cancel, &mut |_| {}).unwrap();
        assert_eq!(once, holder.join("notes.txt.zip"));
        fs::remove_file(&once).unwrap();

        let deep = expand(&outer, &Settings::default(), &cancel, &mut |_| {}).unwrap();
        assert_eq!(deep, holder.join("notes.txt"));
        assert_eq!(fs::read_to_string(&deep).unwrap(), "one");
        assert!(!holder.join("notes.txt.zip").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
