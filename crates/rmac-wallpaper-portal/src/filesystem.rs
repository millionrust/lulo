use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Error, ErrorKind, Operation, RequestId};

pub(crate) const LOCK_FILE: &str = ".portal-import.lock";
pub(crate) const INCOMING_PREFIX: &str = ".incoming-";
const MANAGED_DIRECTORY: &str = "wallpapers";
pub(crate) const HASH_HEX_BYTES: usize = 64;

pub(crate) static AUTHORITY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn managed_root_from_environment() -> Result<PathBuf, Error> {
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/rmac"));
    #[cfg(not(target_os = "macos"))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        })
        .map(|data| data.join("rmac"));
    root.map(|root| root.join(MANAGED_DIRECTORY))
        .ok_or_else(|| {
            failure(
                Operation::EstablishAuthority,
                ErrorKind::InvalidAuthority,
                "the user data directory is unavailable",
            )
        })
}

pub(crate) fn acquire_lease(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        io_error(
            Operation::EstablishAuthority,
            error,
            "open the wallpaper importer lease",
        )
    })?;
    if !file
        .metadata()
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Err(failure(
            Operation::EstablishAuthority,
            ErrorKind::InvalidAuthority,
            "the wallpaper importer lease is not a regular file",
        ));
    }
    // SAFETY: `file` owns a valid descriptor for the lifetime of the call.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = io::Error::last_os_error();
        let kind = if error
            .raw_os_error()
            .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
        {
            ErrorKind::AuthorityBusy
        } else {
            ErrorKind::Io(error.kind())
        };
        return Err(failure(
            Operation::EstablishAuthority,
            kind,
            "another wallpaper importer owns the authority",
        ));
    }
    Ok(file)
}

pub(crate) fn next_request_id() -> Result<RequestId, Error> {
    next_identity(&REQUEST_SEQUENCE, Operation::AdmitRequest).map(RequestId)
}

pub(crate) fn next_identity(counter: &AtomicU64, operation: Operation) -> Result<u64, Error> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| {
            failure(
                operation,
                ErrorKind::InvalidAuthority,
                "wallpaper portal identity space is exhausted",
            )
        })?
        .checked_add(1)
        .ok_or_else(|| {
            failure(
                operation,
                ErrorKind::InvalidAuthority,
                "wallpaper portal identity space is exhausted",
            )
        })
}

pub(crate) fn validate_absolute_path(path: &Path) -> Result<(), Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(failure(
            Operation::EstablishAuthority,
            ErrorKind::InvalidAuthority,
            "wallpaper portal authority paths must be normalized and absolute",
        ));
    }
    Ok(())
}

pub(crate) fn managed_name(
    fingerprint: rmac_storage::FileFingerprint,
    format: rmac_wallpaper_system::ImageFormat,
) -> String {
    let mut name = String::with_capacity(HASH_HEX_BYTES + 5);
    for byte in fingerprint.sha256 {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name.push('.');
    name.push_str(match format {
        rmac_wallpaper_system::ImageFormat::Png => "png",
        rmac_wallpaper_system::ImageFormat::Jpeg => "jpg",
        rmac_wallpaper_system::ImageFormat::WebP => "webp",
    });
    name
}

pub(crate) fn is_managed_name(name: &str) -> bool {
    let Some((hash, extension)) = name.rsplit_once('.') else {
        return false;
    };
    hash.len() == HASH_HEX_BYTES
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && matches!(extension, "png" | "jpg" | "webp")
}

pub(crate) fn referenced_managed_paths(
    wallpaper: &rmac_shell_settings::WallpaperSettings,
    root: &Path,
) -> BTreeSet<PathBuf> {
    std::iter::once(&wallpaper.default)
        .chain(wallpaper.per_output.values())
        .filter_map(|selection| rmac_wallpaper::parse_source(selection.source.as_deref()).ok())
        .filter_map(|source| match source {
            rmac_wallpaper::Source::File(path)
                if path.parent() == Some(root)
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(is_managed_name) =>
            {
                Some(path)
            }
            rmac_wallpaper::Source::BuiltIn(_) | rmac_wallpaper::Source::File(_) => None,
        })
        .collect()
}

pub(crate) fn remove_exact(path: &Path, operation: Operation) -> Result<(), Error> {
    rmac_storage::remove_file_durable(path).map_err(|error| {
        io_error(
            operation,
            error,
            "remove a private wallpaper transaction file",
        )
    })
}

pub(crate) fn remove_if_present(path: &Path, operation: Operation) -> Result<(), Error> {
    match rmac_storage::remove_file_durable(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(
            operation,
            error,
            "remove a private wallpaper transaction file",
        )),
    }
}

pub(crate) fn sync_directory(path: &Path, operation: Operation) -> Result<(), Error> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(operation, error, "sync the wallpaper import directory"))
}

pub(crate) fn io_error(operation: Operation, error: io::Error, detail: &str) -> Error {
    failure(
        operation,
        ErrorKind::Io(error.kind()),
        format!("{detail}: {error}"),
    )
}

pub(crate) fn settings_error(operation: Operation, error: rmac_shell_settings::Error) -> Error {
    failure(operation, ErrorKind::Settings, error.to_string())
}

pub(crate) fn failure(operation: Operation, kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        operation,
        kind,
        detail: detail.into(),
    }
}
