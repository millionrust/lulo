//! The Recycle Bin at the end of the Dock, where the Mac has the Trash:
//! whether it holds anything (for its full or empty picture), opening it
//! and emptying it. Nothing polls: one thread waits on change
//! notifications for each fixed drive's `$Recycle.Bin` folder and counts
//! the bin again only after one fires (`SHQueryRecycleBinW` can take a
//! moment on a large bin, so never on the UI thread).

use std::time::Duration;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FindFirstChangeNotificationW, FindNextChangeNotification, GetDriveTypeW, GetLogicalDrives,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
};
use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHQueryRecycleBinW, SHQUERYRBINFO};

use super::status::StatusEvent;
use super::trace;

const DRIVE_FIXED: u32 = 3;

/// Whether any drive's Recycle Bin holds an item.
pub fn is_full() -> bool {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a sized out-parameter; a null root means every drive.
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.is_ok() && info.i64NumItems > 0
}

/// The fixed drives' recycle folders, which change when anything is
/// recycled, restored or the bin is emptied.
fn bin_folders() -> Vec<String> {
    // SAFETY: no arguments.
    let drives = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| drives & (1 << bit) != 0)
        .map(|bit| char::from(b'A' + bit))
        .filter(|letter| {
            let root = HSTRING::from(format!("{letter}:\\"));
            // SAFETY: a NUL-terminated root path.
            let drive_type = unsafe { GetDriveTypeW(&root) };
            drive_type == DRIVE_FIXED
        })
        .map(|letter| format!("{letter}:\\$Recycle.Bin"))
        .filter(|folder| std::path::Path::new(folder).is_dir())
        .collect()
}

/// Report the bin's state now and whenever it changes.
pub fn watch(sender: async_channel::Sender<StatusEvent>) {
    let spawned = std::thread::Builder::new()
        .name("lulo-recycle-bin".into())
        .spawn(move || {
            let mut full = is_full();
            if sender.send_blocking(StatusEvent::RecycleBin(full)).is_err() {
                return;
            }
            let mut handles: Vec<HANDLE> = Vec::new();
            for folder in bin_folders() {
                // SAFETY: watches a folder for name changes below it.
                let handle = unsafe {
                    FindFirstChangeNotificationW(
                        &HSTRING::from(folder.as_str()),
                        true,
                        FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                    )
                };
                if let Ok(handle) = handle {
                    handles.push(handle);
                }
            }
            trace(|| format!("recycle bin: watching {} drives", handles.len()));
            if handles.is_empty() {
                return;
            }
            loop {
                // SAFETY: waits on the change handles opened above.
                let signalled =
                    unsafe { WaitForMultipleObjects(&handles, false, INFINITE) }.0 as usize;
                if signalled >= handles.len() {
                    return;
                }
                // Recycling many files fires many changes: let them land,
                // then count once.
                std::thread::sleep(Duration::from_millis(300));
                for &handle in &handles {
                    // SAFETY: re-arms each handle (a no-op for one that
                    // has not fired).
                    let _ = unsafe { FindNextChangeNotification(handle) };
                }
                let now = is_full();
                if now != full {
                    full = now;
                    if sender.send_blocking(StatusEvent::RecycleBin(full)).is_err() {
                        return;
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("lulo-shell: the Dock's Recycle Bin will not follow changes: {error}");
    }
}

/// Empty the bin. Windows asks the user first, in its own words.
pub fn empty() {
    let _ = std::thread::Builder::new()
        .name("lulo-empty-bin".into())
        .spawn(|| {
            // SAFETY: a null root empties every drive's bin; flags 0 keep
            // Windows' confirmation, progress and sound.
            let result = unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), 0) };
            trace(|| format!("recycle bin emptied: {}", result.is_ok()));
        });
}
