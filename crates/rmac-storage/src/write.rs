//! Durable private directory and atomic write primitives.

use super::*;

/// Create the final app-owned state directory and enforce owner-only access.
///
/// The caller must already trust the parent authority. The final component is
/// checked before and after creation so an existing link is never accepted as
/// the private directory.
pub fn create_dir_all_private(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private state directory is not a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path)?;
        }
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private state directory changed during creation",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Unlink a file and sync its parent directory before returning success.
pub fn remove_file_durable(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "removal target has no parent directory",
        )
    })?;
    std::fs::remove_file(path)?;
    File::open(parent)?.sync_all()
}

/// Atomically replace `path` using a same-directory temporary file.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent directory",
        )
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Atomically replace a private state file.
///
/// On Unix, the temporary file is created as `0600` before any content is
/// written and remains `0600` after replacement, regardless of umask or prior
/// target permissions. Use this for notification content, recent-item data,
/// and other user-private state—not user-authored documents whose permissions
/// should be preserved.
pub fn atomic_write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent directory",
        )
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private-data");
    let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));

    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Durably create a private state file and never replace an existing path.
///
/// On Unix, the destination is `0600` before any content is written. A write,
/// permission, or sync failure removes only the file created by this call.
/// Callers that retry after an error must still reread the path: an error can
/// be reported after the directory entry became visible and cleanup itself is
/// best effort.
pub fn write_new_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent directory",
        )
    })?;
    let mut created = false;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        created = true;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() && created {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Durably stream a bounded source into a fresh owner-only file.
///
/// The destination is removed on any read, size, write, permission, or sync
/// failure. The caller must still reread after an error because cleanup is
/// necessarily best effort.
pub fn write_new_private_stream(
    path: &Path,
    source: &mut dyn io::Read,
    maximum: u64,
) -> io::Result<FileFingerprint> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent directory",
        )
    })?;
    let mut created = false;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        created = true;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let mut hasher = Sha256::new();
        let mut byte_len = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            byte_len = byte_len
                .checked_add(count as u64)
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
            if byte_len > maximum {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "private stream exceeds its configured limit",
                ));
            }
            hasher.update(&buffer[..count]);
            file.write_all(&buffer[..count])?;
        }
        file.sync_all()?;
        drop(file);
        File::open(parent)?.sync_all()?;
        Ok(FileFingerprint {
            byte_len,
            sha256: hasher.finalize().into(),
        })
    })();

    if result.is_err() && created {
        let _ = remove_file_durable(path);
    }
    result
}

/// Copy to a newly created destination and never overwrite an existing path.
/// A copy/sync/permission failure removes only the destination created by this
/// invocation, leaving no partial attachment behind.
pub fn copy_no_clobber(source: &Path, destination: &Path) -> io::Result<u64> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination has no parent directory",
        )
    })?;
    let mut destination_created = false;
    let result = (|| {
        let mut source_file = File::open(source)?;
        let mut destination_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        destination_created = true;
        let copied = io::copy(&mut source_file, &mut destination_file)?;
        destination_file.set_permissions(source_file.metadata()?.permissions())?;
        destination_file.sync_all()?;
        drop(destination_file);
        File::open(parent)?.sync_all()?;
        Ok(copied)
    })();

    if result.is_err() && destination_created {
        let _ = std::fs::remove_file(destination);
    }
    result
}
