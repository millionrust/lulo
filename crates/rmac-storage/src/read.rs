//! Bounded no-follow reads, fingerprints, and verified streaming copies.

use super::*;

pub(super) fn read_open_file_bounded(file: File, maximum: usize) -> io::Result<Vec<u8>> {
    if file.metadata()?.len() > maximum as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the configured size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
    file.take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds the configured size limit",
        ));
    }
    Ok(bytes)
}

/// Read a regular, singly linked file without following its final path.
pub fn read_bounded_no_follow(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    read_open_file_bounded(open_private_file_no_follow(path)?, maximum)
}

/// Stream a SHA-256 fingerprint from a regular owner-owned, singly linked file
/// without following its final path or allocating its contents in full.
pub fn fingerprint_bounded_no_follow(path: &Path, maximum: u64) -> io::Result<FileFingerprint> {
    let mut file = open_private_file_no_follow(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut byte_len = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        byte_len = byte_len
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file length overflow"))?;
        if byte_len > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file exceeds the configured size limit",
            ));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(FileFingerprint {
        byte_len,
        sha256: hasher.finalize().into(),
    })
}

/// Review a regular, non-symlink destination before an atomic export.
pub fn inspect_destination(path: &Path, maximum: u64) -> io::Result<DestinationBaseline> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DestinationBaseline::Missing),
        Err(error) => Err(error),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "export destination is not a regular file",
            ))
        }
        Ok(_) => {
            fingerprint_bounded_regular_no_follow(path, maximum).map(DestinationBaseline::Exact)
        }
    }
}

/// Stream one private managed file into `destination` only when its complete
/// no-follow fingerprint exactly matches the authoritative record.
pub fn copy_verified_private_file(
    source: &Path,
    expected: FileFingerprint,
    destination: &mut dyn io::Write,
) -> Result<(), VerifiedCopyError> {
    let mut source = open_private_file_no_follow(source)
        .map_err(|error| VerifiedCopyError::Source(error.kind()))?;
    let mut hasher = Sha256::new();
    let mut byte_len = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| VerifiedCopyError::Source(error.kind()))?;
        if count == 0 {
            break;
        }
        byte_len = byte_len
            .checked_add(count as u64)
            .ok_or(VerifiedCopyError::Source(io::ErrorKind::InvalidData))?;
        if byte_len > expected.byte_len {
            return Err(VerifiedCopyError::Source(io::ErrorKind::InvalidData));
        }
        hasher.update(&buffer[..count]);
        destination
            .write_all(&buffer[..count])
            .map_err(|error| VerifiedCopyError::Destination(error.kind()))?;
    }
    let actual = FileFingerprint {
        byte_len,
        sha256: hasher.finalize().into(),
    };
    if actual != expected {
        return Err(VerifiedCopyError::Source(io::ErrorKind::InvalidData));
    }
    Ok(())
}

/// Stream an export into an adjacent temporary file, recheck the reviewed
/// destination immediately before replacement, and verify the final bytes.
///
/// The producer never writes directly to the selected path. Any producer or
/// preflight failure removes only the temporary file.
pub fn atomic_write_stream_checked<F>(
    path: &Path,
    baseline: DestinationBaseline,
    maximum_bytes: u64,
    producer: F,
) -> io::Result<FileFingerprint>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "export target has no parent directory",
        )
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export");
    let temporary = parent.join(format!(
        ".{name}.rmac-export-{}-{sequence}",
        std::process::id()
    ));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        producer(&mut file)?;
        file.sync_all()?;
        drop(file);
        let output = fingerprint_bounded_regular_no_follow(&temporary, maximum_bytes)?;
        if inspect_destination(path, maximum_bytes)? != baseline {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "export destination changed after review",
            ));
        }
        if let DestinationBaseline::Exact(_) = baseline {
            let permissions = std::fs::metadata(path)?.permissions();
            std::fs::set_permissions(&temporary, permissions)?;
        }
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        let readback = fingerprint_bounded_regular_no_follow(path, output.byte_len)?;
        if readback != output {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "export destination readback did not match",
            ));
        }
        Ok(output)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Open a user-selected regular file without following its final path.
///
/// Unlike private-state helpers this intentionally does not require ownership
/// or a single hard link: a portal-selected ordinary file may legitimately be
/// shared, but its final component must never be a symlink or non-file.
pub fn open_regular_no_follow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "refusing to follow an export destination link",
            ));
        }
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "export destination is not a regular file",
        ));
    }
    Ok(file)
}

/// Stream a bounded fingerprint from a user-selected ordinary file without
/// following its final path.
pub fn fingerprint_bounded_regular_no_follow(
    path: &Path,
    maximum: u64,
) -> io::Result<FileFingerprint> {
    let mut file = open_regular_no_follow(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut byte_len = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        byte_len = byte_len
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file length overflow"))?;
        if byte_len > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file exceeds the configured size limit",
            ));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(FileFingerprint {
        byte_len,
        sha256: hasher.finalize().into(),
    })
}

pub(super) fn open_private_file_no_follow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "refusing to follow a private state link",
            ));
        }
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private state is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private state has multiple filesystem links",
            ));
        }
        // SAFETY: geteuid has no preconditions and does not mutate state.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private state is owned by another user",
            ));
        }
    }
    Ok(file)
}
