use std::io;
use std::path::{Path, PathBuf};

#[cfg(all(
    unix,
    not(target_os = "macos"),
    not(target_os = "ios"),
    not(target_os = "android")
))]
use std::ffi::OsStr;

use crate::{Backend, TrashEntryId};

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
