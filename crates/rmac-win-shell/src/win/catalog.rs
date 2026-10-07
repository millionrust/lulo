//! What Spotlight can find: the Lulo apps, every app in Windows' Apps
//! folder (Start menu shortcuts and Store apps alike), and the files in the
//! user's own folders (Desktop, Documents, Downloads, Pictures, Music,
//! Videos). Each list is read once on a background thread and read again
//! only after Windows reports a change in the folders it came from; no
//! indexer runs and nothing polls.

use std::path::{Path, PathBuf};

use windows::core::{GUID, PWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, IBindCtx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_CommonPrograms, FOLDERID_Desktop,
    FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Programs,
    FOLDERID_Videos, IEnumShellItems, IShellItem, SHGetKnownFolderItem, SHGetKnownFolderPath,
    KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

use crate::model::apps::LULO_APPS;
use crate::model::search::{Entry, Kind, Target};

/// At most this many files and folders are kept for Spotlight.
const MAX_FILES: usize = 40_000;
/// How deep below each folder Spotlight looks.
const MAX_DEPTH: usize = 5;

/// Which list a change notification makes stale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum List {
    Apps,
    Files,
}

/// Initialise COM on the calling thread (the catalogue's own thread).
pub fn init_com() {
    // SAFETY: once per thread, before any COM call on it.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
}

/// A string Windows allocated with `CoTaskMemAlloc`, copied and freed.
fn take_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: `value` is a NUL-terminated string this function now owns.
    unsafe {
        let text = String::from_utf16_lossy(value.as_wide());
        CoTaskMemFree(Some(value.0 as *const core::ffi::c_void));
        text
    }
}

pub fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: the returned string is freed by `take_string`.
    let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    let path = take_string(path);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The user's own folders Spotlight searches, with the names it shows.
fn file_roots() -> Vec<(PathBuf, &'static str)> {
    [
        (&FOLDERID_Desktop, "Desktop"),
        (&FOLDERID_Documents, "Documents"),
        (&FOLDERID_Downloads, "Downloads"),
        (&FOLDERID_Pictures, "Pictures"),
        (&FOLDERID_Music, "Music"),
        (&FOLDERID_Videos, "Videos"),
    ]
    .into_iter()
    .filter_map(|(id, name)| known_folder(id).map(|path| (path, name)))
    .collect()
}

fn app_roots() -> Vec<PathBuf> {
    [&FOLDERID_Programs, &FOLDERID_CommonPrograms]
        .into_iter()
        .filter_map(known_folder)
        .collect()
}

/// Every app Spotlight can open. Needs COM on the calling thread.
pub fn load_apps() -> Vec<Entry> {
    let mut apps = LULO_APPS
        .iter()
        .map(|app| Entry::new(app.name, Target::Lulo(app.exe), "", Kind::Application))
        .collect::<Vec<_>>();
    match apps_folder() {
        Some(found) if !found.is_empty() => {
            super::trace(|| format!("catalog: {} apps in the Apps folder", found.len()));
            apps.extend(found);
        }
        _ => {
            let found = start_menu_shortcuts();
            super::trace(|| {
                format!(
                    "catalog: the Apps folder could not be read; {} Start menu shortcuts",
                    found.len()
                )
            });
            apps.extend(found);
        }
    }
    apps
}

/// Windows' Apps folder: the Start menu's shortcuts and the Store apps,
/// each opened through `shell:AppsFolder\<its parsing name>`.
fn apps_folder() -> Option<Vec<Entry>> {
    // SAFETY: COM calls on interfaces this function owns.
    unsafe {
        let folder: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None).ok()?;
        let items: IEnumShellItems = folder
            .BindToHandler(None::<&IBindCtx>, &BHID_EnumItems)
            .ok()?;
        let mut found = Vec::new();
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else {
                break;
            };
            let name = item
                .GetDisplayName(SIGDN_NORMALDISPLAY)
                .map(take_string)
                .unwrap_or_default();
            let parsing = item
                .GetDisplayName(SIGDN_PARENTRELATIVEPARSING)
                .map(take_string)
                .unwrap_or_default();
            if name.is_empty() || parsing.is_empty() {
                continue;
            }
            found.push(Entry::new(
                name,
                Target::Shell(format!("shell:AppsFolder\\{parsing}")),
                "",
                Kind::Application,
            ));
        }
        Some(found)
    }
}

/// The Start menu's shortcuts, when the Apps folder cannot be read.
fn start_menu_shortcuts() -> Vec<Entry> {
    let mut found = Vec::new();
    for root in app_roots() {
        walk(&root, 0, 4, &mut |path, is_dir| {
            if is_dir {
                return;
            }
            let is_link = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                return;
            };
            if is_link && !name.to_lowercase().contains("uninstall") {
                found.push(Entry::new(
                    name,
                    Target::Shell(path.to_string_lossy().into_owned()),
                    "",
                    Kind::Application,
                ));
            }
        });
    }
    found
}

/// The files and folders in the user's own folders.
pub fn load_files() -> Vec<Entry> {
    let mut found = Vec::new();
    for (root, label) in file_roots() {
        walk(&root, 0, MAX_DEPTH, &mut |path, is_dir| {
            if found.len() >= MAX_FILES {
                return;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return;
            };
            let location = path
                .parent()
                .and_then(|parent| parent.strip_prefix(&root).ok())
                .map(|relative| {
                    let relative = relative.to_string_lossy();
                    if relative.is_empty() {
                        label.to_owned()
                    } else {
                        format!("{label}\\{relative}")
                    }
                })
                .unwrap_or_else(|| label.to_owned());
            found.push(Entry::new(
                name,
                Target::Shell(path.to_string_lossy().into_owned()),
                location,
                if is_dir { Kind::Folder } else { Kind::Document },
            ));
        });
    }
    found
}

fn hidden(path: &Path, metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini"))
        || metadata.file_attributes() & (HIDDEN | SYSTEM) != 0
}

fn walk(directory: &Path, depth: usize, max_depth: usize, visit: &mut dyn FnMut(&Path, bool)) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Links (junctions such as "My Music") are not followed.
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if hidden(&path, &metadata) || metadata.file_type().is_symlink() {
            continue;
        }
        let is_dir = metadata.is_dir();
        visit(&path, is_dir);
        if is_dir && depth + 1 < max_depth {
            walk(&path, depth + 1, max_depth, visit);
        }
    }
}

/// Report `List::Apps` or `List::Files` whenever a name changes in the
/// folders that list was read from. One thread blocks on Windows' change
/// notifications for all of them.
pub fn watch(on_change: impl Fn(List) + Send + 'static) {
    let folders = app_roots()
        .into_iter()
        .map(|path| (path, List::Apps))
        .chain(
            file_roots()
                .into_iter()
                .map(|(path, _)| (path, List::Files)),
        )
        .collect::<Vec<_>>();
    let spawned = std::thread::Builder::new()
        .name("lulo-catalog-watch".into())
        .spawn(move || {
            let mut handles: Vec<HANDLE> = Vec::new();
            let mut lists: Vec<List> = Vec::new();
            for (path, list) in folders {
                // SAFETY: watches a folder for name changes below it.
                let handle = unsafe {
                    FindFirstChangeNotificationW(
                        &windows::core::HSTRING::from(path.as_os_str()),
                        true,
                        FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                    )
                };
                if let Ok(handle) = handle {
                    handles.push(handle);
                    lists.push(list);
                }
            }
            if handles.is_empty() {
                return;
            }
            loop {
                // SAFETY: waits on the change handles opened above.
                let signalled =
                    unsafe { WaitForMultipleObjects(&handles, false, INFINITE) }.0 as usize;
                let Some(&list) = lists.get(signalled) else {
                    return;
                };
                on_change(list);
                // SAFETY: re-arms the handle that fired.
                if unsafe { FindNextChangeNotification(handles[signalled]) }.is_err() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("lulo-shell: Spotlight will not notice new apps or files: {error}");
    }
}
