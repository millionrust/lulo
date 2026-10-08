//! The Desktop folder as Lulo mode's desktop shows it (ADR 0023 "Lulo
//! mode"): what is in it, and the Finder's desktop commands done the
//! Windows way. Listing reuses Lulo OS's desktop model (`rmac-desktop`);
//! Windows supplies the rest: the user's and the public Desktop folders,
//! hidden and system items left out as Explorer leaves them out, renames
//! that never replace an existing item (`MoveFileExW` without
//! `MOVEFILE_REPLACE_EXISTING`), the Recycle Bin (`SHFileOperationW` with
//! undo), shortcuts (`IShellLinkW`) and the Properties sheet. Everything
//! here runs off the UI thread.

use std::path::{Path, PathBuf};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::{
    FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME,
};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_PublicDesktop, SHFileOperationW, SHObjectProperties, FOF_ALLOWUNDO,
    FOF_NOCONFIRMMKDIR, FO_DELETE, SHFILEOPSTRUCTW, SHOP_FILEPATH,
};

use super::{catalog, trace};

/// One icon on the desktop.
#[derive(Clone, Debug, PartialEq)]
pub struct DesktopItem {
    pub path: PathBuf,
    pub name: String,
    pub is_folder: bool,
    /// From the public Desktop, shared by every user of the PC (renaming
    /// or deleting it may need an administrator).
    pub shared: bool,
    pub modified_millis: u128,
    pub size_bytes: u64,
}

/// The user's Desktop folder, where new items go.
pub fn user_desktop() -> Option<PathBuf> {
    catalog::known_folder(&FOLDERID_Desktop)
}

fn folders() -> Vec<(PathBuf, bool)> {
    let mut folders = Vec::new();
    if let Some(user) = user_desktop() {
        folders.push((user, false));
    }
    if let Some(public) = catalog::known_folder(&FOLDERID_PublicDesktop) {
        folders.push((public, true));
    }
    folders
}

/// Hidden and system items stay off the desktop, as Explorer keeps them
/// off (`desktop.ini` above all).
fn hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("desktop.ini"))
        || std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_attributes() & (HIDDEN | SYSTEM) != 0)
}

/// What the desktop shows: the user's Desktop, then the public one's items
/// (the shortcuts installers put there for everyone), each by name.
pub fn list() -> Vec<DesktopItem> {
    let mut items = Vec::new();
    for (folder, shared) in folders() {
        let Ok(snapshot) = rmac_desktop::scan(&folder, rmac_desktop::SortOrder::Name) else {
            continue;
        };
        items.extend(
            snapshot
                .items
                .into_iter()
                .filter(|item| !hidden(&item.path))
                .map(|item| DesktopItem {
                    is_folder: item.kind == rmac_desktop::ItemKind::Directory,
                    name: display_name(&item.name),
                    path: item.path,
                    shared,
                    modified_millis: item.modified_millis,
                    size_bytes: item.size_bytes,
                }),
        );
    }
    items
}

/// The name the desktop shows: a shortcut without its `.lnk` or `.url`,
/// as Explorer shows it.
pub fn display_name(file_name: &str) -> String {
    let lower = file_name.to_ascii_lowercase();
    for extension in [".lnk", ".url"] {
        if lower.ends_with(extension) && file_name.len() > extension.len() {
            return file_name[..file_name.len() - extension.len()].to_owned();
        }
    }
    file_name.to_owned()
}

/// Call `on_change` (from one parked thread) whenever a name changes in
/// either Desktop folder.
pub fn watch(on_change: impl Fn() + Send + 'static) {
    let folders = folders();
    let spawned = std::thread::Builder::new()
        .name("lulo-desktop-watch".into())
        .spawn(move || {
            let handles = folders
                .iter()
                .filter_map(|(folder, _)| {
                    // SAFETY: watches a folder for name changes in it.
                    unsafe {
                        FindFirstChangeNotificationW(
                            &HSTRING::from(folder.as_os_str()),
                            false,
                            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                        )
                    }
                    .ok()
                })
                .collect::<Vec<_>>();
            if handles.is_empty() {
                return;
            }
            loop {
                // SAFETY: waits on the handles opened above.
                let signalled =
                    unsafe { WaitForMultipleObjects(&handles, false, INFINITE) }.0 as usize;
                let Some(&handle) = handles.get(signalled) else {
                    return;
                };
                on_change();
                // SAFETY: re-arms the handle that fired.
                if unsafe { FindNextChangeNotification(handle) }.is_err() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("lulo-shell: the desktop will not notice new files: {error}");
    }
}

/// A NUL-separated, double-NUL-terminated list of paths, as
/// `SHFileOperationW` reads them.
fn path_list(paths: &[PathBuf]) -> Vec<u16> {
    let mut list = Vec::new();
    for path in paths {
        list.extend(path.as_os_str().to_string_lossy().encode_utf16());
        list.push(0);
    }
    list.push(0);
    list
}

/// Move `paths` to the Recycle Bin, with undo, as Explorer's Delete does.
pub fn recycle(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return true;
    }
    let from = path_list(paths);
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0) as u16,
        ..Default::default()
    };
    // SAFETY: `from` outlives the call and is double-NUL-terminated.
    let result = unsafe { SHFileOperationW(&mut operation) };
    trace(|| format!("desktop recycled {} item(s): {result}", paths.len()));
    result == 0 && !operation.fAnyOperationsAborted.as_bool()
}

/// Move `source` to `destination` without ever replacing an item there.
pub fn move_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    // SAFETY: NUL-terminated paths that outlive the call. Without
    // MOVEFILE_REPLACE_EXISTING Windows refuses when the name is taken.
    unsafe {
        MoveFileExW(
            &HSTRING::from(source),
            &HSTRING::from(destination),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF))
}

/// Rename a desktop item. A shortcut keeps its hidden `.lnk` extension.
pub fn rename(item: &DesktopItem, new_name: &str) -> Result<PathBuf, rmac_desktop::RenameError> {
    let old_name = item
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(rmac_desktop::RenameError::Invalid)?;
    let hidden_extension = &old_name[item.name.len().min(old_name.len())..];
    let new_file_name = format!("{new_name}{hidden_extension}");
    match rmac_desktop::rename::check_name(old_name, &new_file_name) {
        rmac_desktop::rename::NameCheck::Unchanged => return Ok(item.path.clone()),
        rmac_desktop::rename::NameCheck::Empty | rmac_desktop::rename::NameCheck::Invalid => {
            return Err(rmac_desktop::RenameError::Invalid)
        }
        _ => {}
    }
    if new_file_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(rmac_desktop::RenameError::Invalid);
    }
    let target = item
        .path
        .parent()
        .ok_or(rmac_desktop::RenameError::Invalid)?
        .join(&new_file_name);
    let case_only = old_name.to_lowercase() == new_file_name.to_lowercase();
    if !case_only && std::fs::symlink_metadata(&target).is_ok() {
        return Err(rmac_desktop::RenameError::Taken);
    }
    move_no_replace(&item.path, &target)
        .map(|()| target)
        .map_err(|error| match error.raw_os_error() {
            // ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS
            Some(183) | Some(80) => rmac_desktop::RenameError::Taken,
            _ => rmac_desktop::RenameError::Io(error.kind()),
        })
}

/// Move `items` into the folder `folder` (a drop onto a folder icon).
pub fn move_into(items: &[PathBuf], folder: &Path) -> usize {
    items
        .iter()
        .filter(|item| item.as_path() != folder && !folder.starts_with(item))
        .filter_map(|item| {
            let name = item.file_name()?;
            move_no_replace(item, &folder.join(name)).ok()
        })
        .count()
}

/// File ▸ New Folder on the desktop: "untitled folder", as the Finder
/// names it.
pub fn new_folder() -> Option<PathBuf> {
    rmac_desktop::create_folder(&user_desktop()?).ok()
}

/// File ▸ Duplicate: "name copy" beside the original.
pub fn duplicate(path: &Path) -> Option<PathBuf> {
    rmac_desktop::duplicate_file(path).ok()
}

/// Create Shortcut (the Mac's Make Alias): a real `.lnk` beside the item,
/// as Explorer makes.
pub fn make_shortcut(path: &Path) -> Option<PathBuf> {
    use windows::core::Interface as _;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let name = path.file_name()?.to_string_lossy().into_owned();
    let parent = path.parent()?;
    let destination = (1..1000u32)
        .map(|index| match index {
            1 => parent.join(format!("{name} - Shortcut.lnk")),
            _ => parent.join(format!("{name} - Shortcut ({index}).lnk")),
        })
        .find(|candidate| std::fs::symlink_metadata(candidate).is_err())?;
    // SAFETY: COM set-up on this (worker) thread, balanced below; the link
    // object is dropped before COM is uninitialised; the strings outlive
    // each call.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let saved = CoCreateInstance::<_, IShellLinkW>(
            &ShellLink,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
        .and_then(|link| {
            link.SetPath(&HSTRING::from(path))?;
            link.SetWorkingDirectory(&HSTRING::from(parent))?;
            let file: IPersistFile = link.cast()?;
            file.Save(&HSTRING::from(destination.as_path()), true)
        });
        if initialized {
            CoUninitialize();
        }
        saved.ok().map(|()| destination)
    }
}

/// Get Info: Windows' Properties sheet for the item.
pub fn properties(path: &Path) {
    // SAFETY: a NUL-terminated path; the sheet runs on its own.
    let _ =
        unsafe { SHObjectProperties(None, SHOP_FILEPATH, &HSTRING::from(path), PCWSTR::null()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_show_without_their_extension() {
        assert_eq!(display_name("Google Chrome.lnk"), "Google Chrome");
        assert_eq!(display_name("Site.URL"), "Site");
        assert_eq!(display_name("notes.txt"), "notes.txt");
        assert_eq!(display_name(".lnk"), ".lnk");
    }

    #[test]
    fn path_lists_end_with_two_nuls() {
        let list = path_list(&[PathBuf::from(r"C:\a"), PathBuf::from(r"C:\b")]);
        assert_eq!(&list[list.len() - 2..], &[0, 0]);
        assert_eq!(list.iter().filter(|&&unit| unit == 0).count(), 3);
    }
}
