//! The Trash on Windows is the Recycle Bin (ADR 0023, "Phase 3 revised:
//! shared shell views"): its item count from `SHQueryRecycleBinW`, emptying
//! through `SHEmptyRecycleBinW`, and change notifications from the user's
//! own `$Recycle.Bin\<SID>` folder on each fixed drive. None of these loads
//! the shell's folder views into the Dock's process, unlike enumerating the
//! bin's items through COM.

use std::path::PathBuf;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    SHEmptyRecycleBinW, SHQueryRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
    SHERB_NOSOUND, SHQUERYRBINFO,
};

/// `DRIVE_FIXED`.
const FIXED: u32 = 3;

/// How many items the Recycle Bin holds, across every drive.
pub(crate) fn item_count() -> Result<usize, String> {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a sized out-parameter; no root path means every drive.
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }
        .map_err(|error| format!("the Recycle Bin could not be read: {error}"))?;
    Ok(usize::try_from(info.i64NumItems.max(0)).unwrap_or(usize::MAX))
}

/// Empty the Recycle Bin. The Dock asked the user first, as the Mac does,
/// so Windows' own confirmation, progress and sound are not shown again.
pub(crate) fn empty() -> Result<(), String> {
    // SAFETY: no owner window and no root path (every drive).
    unsafe {
        SHEmptyRecycleBinW(
            None,
            PCWSTR::null(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    }
    .map_err(|error| format!("the Recycle Bin could not be emptied: {error}"))
}

/// The signed-in user's SID as text (`S-1-5-21-…`).
fn user_sid() -> Option<String> {
    // SAFETY: the token is closed; the buffer is sized by the first call
    // and the SID string is freed with LocalFree.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut size = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
        let mut buffer = vec![0u8; size as usize];
        let read = GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        );
        let _ = CloseHandle(token);
        read.ok()?;
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

/// The user's own Recycle Bin folder on each fixed drive, to watch.
pub(crate) fn folders() -> Vec<PathBuf> {
    let Some(sid) = user_sid() else {
        return Vec::new();
    };
    // SAFETY: no arguments.
    let drives = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|index| drives & (1 << index) != 0)
        .map(|index| char::from(b'A' + index))
        .filter(|letter| {
            let root: Vec<u16> = format!("{letter}:\\")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: a NUL-terminated root path.
            unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == FIXED }
        })
        .map(|letter| PathBuf::from(format!("{letter}:\\$Recycle.Bin\\{sid}")))
        .filter(|folder| folder.is_dir())
        .collect()
}
