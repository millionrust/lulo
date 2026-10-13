//! The few places expanding and compressing touch the operating system
//! rather than std: how a new file is opened, folder modes, links, free space
//! and which entry names are safe to create.
//!
//! Unix opens files with `O_NOFOLLOW` and keeps POSIX modes and symbolic
//! links. Windows has neither; there a new file is opened with `create_new`
//! (which refuses any existing name, links included), archive entries that are
//! symbolic links are left out rather than recreated, and names Windows
//! treats specially (`C:`, `file:stream`, `CON`, `NUL.txt`, a trailing dot or
//! space) are refused rather than written.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};

    pub fn create_new_file(path: &Path, mode: u32) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    pub fn create_private_dir(path: &Path) -> io::Result<()> {
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }

    pub fn publish_dir(path: &Path) -> io::Result<()> {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
    }

    pub fn mode_of(metadata: &std::fs::Metadata) -> u32 {
        metadata.permissions().mode() & 0o7777
    }

    pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    pub fn available_space(path: &Path) -> io::Result<u64> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: `path` is NUL-terminated and `space` is valid writable storage.
        let mut space: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers remain valid for the duration of the call.
        if unsafe { libc::statvfs(path.as_ptr(), &mut space) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((space.f_bavail as u64).saturating_mul(space.f_frsize as u64))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    pub fn create_new_file(path: &Path, _mode: u32) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }

    pub fn open_for_reading(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    pub fn create_private_dir(path: &Path) -> io::Result<()> {
        // The per-user folder's own ACL is the privacy boundary (ADR 0023).
        std::fs::create_dir(path)
    }

    pub fn publish_dir(_path: &Path) -> io::Result<()> {
        Ok(())
    }

    pub fn mode_of(metadata: &std::fs::Metadata) -> u32 {
        if metadata.is_dir() {
            0o755
        } else {
            0o644
        }
    }

    /// Windows archives are made without links, and links in an archive are
    /// not recreated: making one needs a privilege most users do not hold.
    pub fn symlink(_target: &Path, _link: &Path) -> io::Result<()> {
        Ok(())
    }

    pub fn available_space(path: &Path) -> io::Result<u64> {
        let wide: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt as _;
            path.as_os_str().encode_wide().chain(Some(0)).collect()
        };
        let mut available = 0u64;
        // SAFETY: `wide` is NUL-terminated and `available` is valid storage.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None) }
            .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))?;
        Ok(available)
    }
}

pub(crate) use imp::*;

/// True when creating an entry with this name would do something other than
/// make an ordinary file or folder of that name. Always false off Windows
/// (`name` there is just bytes).
pub(crate) fn unsafe_component(name: &OsStr) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let Some(name) = name.to_str() else {
        return true;
    };
    windows_unsafe(name)
}

/// The Windows rules, kept platform-neutral so they are tested everywhere.
pub(crate) fn windows_unsafe(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/')
        })
    {
        return true;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        upper
            .strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest.chars().next(), Some('1'..='9')) && rest.len() == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::windows_unsafe;

    #[test]
    fn windows_special_names_are_refused() {
        for bad in [
            "CON",
            "con",
            "NUL.txt",
            "aux.tar.gz",
            "COM1",
            "lpt9.log",
            "a:b",
            "C:",
            "x|y",
            "dir.",
            "name ",
            "",
            "a?b",
            "stream:$DATA",
        ] {
            assert!(windows_unsafe(bad), "{bad:?} should be refused");
        }
    }

    #[test]
    fn ordinary_names_are_kept() {
        for good in [
            "report.pdf",
            "Archive",
            "COM10",
            "console",
            "nul0",
            "a b",
            ".hidden",
            "über.txt",
        ] {
            assert!(!windows_unsafe(good), "{good:?} should be kept");
        }
    }
}
