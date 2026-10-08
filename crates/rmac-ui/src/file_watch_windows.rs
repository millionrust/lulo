//! Windows: tell an app when one file it reads is replaced, with no polling.
//!
//! Lulo OS's apps follow System Settings' theme file with `notify`, whose
//! Windows backend wakes its thread ten times a second for as long as it
//! watches (ADR 0025, docs/parity.md WIN-OS-15). Here one thread instead
//! blocks in `ReadDirectoryChangesW` on the file's folder, which costs
//! nothing until Windows reports a change there, and passes on only the
//! changes that name the file (an atomic save renames a new copy over it).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use windows::core::HSTRING;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// Watch `path` and send one `()` on `changed` after each change to it. A
/// change that arrives while an earlier one is still unread folds into it.
/// The thread ends when `changed` closes (after the next change) or when
/// Windows stops reporting the folder.
pub(crate) fn watch_file(path: &Path, changed: async_channel::Sender<()>) -> std::io::Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the watched file has no folder"))?
        .to_path_buf();
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the watched path has no file name"))?
        .to_os_string();
    std::fs::create_dir_all(&folder)?;
    std::thread::Builder::new()
        .name("lulo file watch".into())
        .spawn(move || watch(folder, name, changed))?;
    Ok(())
}

fn watch(folder: PathBuf, name: OsString, changed: async_channel::Sender<()>) {
    // SAFETY: a folder handle for change reports, closed below.
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(folder.as_os_str()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };
    let Ok(handle) = handle else {
        return;
    };
    // `u32`s keep the records' DWORD alignment.
    let mut buffer = vec![0u32; 2048];
    let name = name.to_string_lossy().to_lowercase();
    loop {
        let mut returned = 0u32;
        // SAFETY: a synchronous read into `buffer`, which outlives it.
        let read = unsafe {
            ReadDirectoryChangesW(
                handle,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * std::mem::size_of::<u32>()) as u32,
                false,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                Some(&mut returned),
                None,
                None,
            )
        };
        if read.is_err() {
            break;
        }
        // Nothing returned means the report overflowed: the file may have
        // changed among the lost records.
        let bytes = buffer_bytes(&buffer, returned as usize);
        let hit = returned == 0 || changed_names(bytes).any(|changed| changed == name);
        if hit && changed.try_send(()).is_err() && changed.is_closed() {
            break;
        }
    }
    // SAFETY: the handle opened above.
    let _ = unsafe { CloseHandle(handle) };
}

fn buffer_bytes(buffer: &[u32], length: usize) -> &[u8] {
    // SAFETY: a `u32` slice viewed as its own bytes.
    let bytes = unsafe {
        std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), std::mem::size_of_val(buffer))
    };
    &bytes[..length.min(bytes.len())]
}

/// The lower-cased file names in a `FILE_NOTIFY_INFORMATION` chain:
/// `NextEntryOffset`, `Action`, `FileNameLength` (bytes), then the name.
fn changed_names(bytes: &[u8]) -> impl Iterator<Item = String> + '_ {
    let mut offset = Some(0usize);
    std::iter::from_fn(move || {
        let start = offset?;
        let field = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                bytes.get(start + at..start + at + 4)?.try_into().ok()?,
            ))
        };
        let next = field(0)? as usize;
        let length = field(8)? as usize;
        let units: Vec<u16> = bytes
            .get(start + 12..start + 12 + length)?
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        offset = (next != 0).then_some(start + next);
        Some(String::from_utf16_lossy(&units).to_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(next: u32, name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&next.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&((units.len() * 2) as u32).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_chain_of_records_yields_each_name() {
        let mut first = record(0, "theme.json.tmp");
        let padded = first.len().div_ceil(4) * 4;
        first.resize(padded, 0);
        first[..4].copy_from_slice(&(padded as u32).to_le_bytes());
        first.extend(record(0, "Theme.json"));
        let names: Vec<_> = changed_names(&first).collect();
        assert_eq!(names, ["theme.json.tmp", "theme.json"]);
    }

    #[test]
    fn a_short_report_yields_nothing_rather_than_panicking() {
        assert_eq!(changed_names(&[1, 2, 3]).count(), 0);
    }

    #[test]
    fn a_saved_file_is_reported() {
        let folder = std::env::temp_dir().join(format!("lulo-file-watch-{}", std::process::id()));
        let path = folder.join("theme.json");
        let (sender, receiver) = async_channel::bounded(1);
        watch_file(&path, sender).unwrap();
        // Give the thread a moment to start reading the folder.
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::write(folder.join("other.json"), b"{}").unwrap();
        std::fs::write(&path, b"{}").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while receiver.try_recv().is_err() {
            assert!(std::time::Instant::now() < deadline, "no change reported");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
