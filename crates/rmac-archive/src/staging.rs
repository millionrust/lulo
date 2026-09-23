//! Private scratch files, no-replace renames and metered reads shared by
//! expanding and compressing.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::naming::unique_path;
use crate::{cancelled_io, display_name, Error, Progress};

/// Report progress at most every 1 MiB (and at the end).
const REPORT_STEP: u64 = 1024 * 1024;
const MAX_PLACE_ATTEMPTS: usize = 64;

/// A private folder or file beside the archive, removed on drop unless
/// kept.
pub(crate) struct Scratch {
    pub(crate) path: PathBuf,
    directory: bool,
    pub(crate) kept: bool,
}

impl Scratch {
    fn name(archive: &Path, role: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        format!(
            ".{}.rmac-{role}-{}-{nanos}",
            display_name(archive),
            std::process::id()
        )
    }

    pub(crate) fn directory(parent: &Path, archive: &Path, role: &str) -> io::Result<Self> {
        let path = parent.join(Self::name(archive, role));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self {
            path,
            directory: true,
            kept: false,
        })
    }

    pub(crate) fn file(parent: &Path, archive: &Path, role: &str) -> io::Result<Self> {
        Ok(Self {
            path: parent.join(Self::name(archive, role)),
            directory: false,
            kept: false,
        })
    }

    /// Move the result into `parent` under the Mac's name.
    pub(crate) fn finish(mut self, parent: &Path, stem: &str) -> Result<PathBuf, Error> {
        let mut children = fs::read_dir(&self.path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        if children.len() == 1 {
            let child = children.remove(0);
            let name = display_name(&child);
            return place(&child, parent, &name);
        }
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o755))?;
        let placed = place(&self.path, parent, stem)?;
        self.kept = true;
        Ok(placed)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        let _ = if self.directory {
            fs::remove_dir_all(&self.path)
        } else {
            fs::remove_file(&self.path)
        };
    }
}

/// Rename `source` to the first free Mac-style name in `directory`, never
/// replacing anything that appeared in the meantime.
pub(crate) fn place(source: &Path, directory: &Path, name: &str) -> Result<PathBuf, Error> {
    for _ in 0..MAX_PLACE_ATTEMPTS {
        let candidate = unique_path(directory, name);
        match rename_noreplace(source, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists).into())
}

#[cfg(target_os = "linux")]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let from_c = CString::new(from.as_os_str().as_bytes())?;
    let to_c = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both pointers are valid NUL-terminated strings for the call.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::ENOSYS) => rename_checked(from, to),
        _ => Err(error),
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let from_c = CString::new(from.as_os_str().as_bytes())?;
    let to_c = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both pointers are valid NUL-terminated strings for the call.
    let result = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOTSUP) | Some(libc::EINVAL) => rename_checked(from, to),
        _ => Err(error),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    rename_checked(from, to)
}

fn rename_checked(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    fs::rename(from, to)
}

/// Progress and cancellation for one pass over a file.
pub(crate) struct Meter<'a> {
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(Progress),
    total: u64,
    /// Reported progress = base + read × numerator ÷ denominator.
    pub(crate) base: u64,
    pub(crate) numerator: u64,
    pub(crate) denominator: u64,
    read: u64,
    reported: Option<u64>,
}

impl<'a> Meter<'a> {
    pub(crate) fn new(
        cancel: &'a AtomicBool,
        progress: &'a mut dyn FnMut(Progress),
        total: u64,
    ) -> Self {
        Self {
            cancel,
            progress,
            total,
            base: 0,
            numerator: 1,
            denominator: 1,
            read: 0,
            reported: None,
        }
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            Err(cancelled_io())
        } else {
            Ok(())
        }
    }

    pub(crate) fn add(&mut self, bytes: u64) {
        self.read = self.read.saturating_add(bytes);
        let scaled = u128::from(self.read) * u128::from(self.numerator)
            / u128::from(self.denominator.max(1));
        let done = self
            .base
            .saturating_add(u64::try_from(scaled).unwrap_or(u64::MAX))
            .min(self.total);
        let due = match self.reported {
            None => true,
            Some(last) => done >= last.saturating_add(REPORT_STEP) || done == self.total,
        };
        if due && self.reported != Some(done) {
            self.reported = Some(done);
            (self.progress)(Progress {
                done,
                total: self.total,
            });
        }
    }
}

/// A reader that meters what passes through it and stops when cancelled.
pub(crate) struct Counted<'a, R> {
    inner: R,
    meter: Meter<'a>,
}

impl<'a, R> Counted<'a, R> {
    pub(crate) fn new(inner: R, meter: Meter<'a>) -> Self {
        Self { inner, meter }
    }
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.meter.check()?;
        let read = self.inner.read(buffer)?;
        self.meter.add(read as u64);
        Ok(read)
    }
}

impl<R: Seek> Seek for Counted<'_, R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
