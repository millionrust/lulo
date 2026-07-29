use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::Duration;

use super::{Error, ErrorKind, Operation};

const ATTEMPTS: usize = 200;
const RETRY: Duration = Duration::from_millis(5);

pub(super) struct FileLock {
    _file: File,
}

impl FileLock {
    #[cfg(unix)]
    pub(super) fn acquire(path: &Path) -> Result<Self, Error> {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
        use std::os::unix::io::AsRawFd as _;

        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let file = options
            .open(path)
            .map_err(|error| Error::io(Operation::Lock, error))?;
        let metadata = file
            .metadata()
            .map_err(|error| Error::io(Operation::Lock, error))?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(Error::new(Operation::Lock, ErrorKind::Invalid));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| Error::io(Operation::Lock, error))?;

        for _ in 0..ATTEMPTS {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(Error::io(Operation::Lock, error));
            }
            std::thread::sleep(RETRY);
        }
        Err(Error::new(Operation::Lock, ErrorKind::Busy))
    }

    #[cfg(not(unix))]
    pub(super) fn acquire(_path: &Path) -> Result<Self, Error> {
        Err(Error::new(Operation::Lock, ErrorKind::Invalid))
    }
}
