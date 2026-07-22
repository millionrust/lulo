//! Durable transaction core for the rmac XDG Wallpaper portal backend.
//!
//! The D-Bus adapter and preview window remain outside this crate. This layer
//! admits one frontend-authenticated request, stages and decodes its local
//! document, then applies an explicitly accepted whole-desktop choice without
//! leaving failed or cancelled imports behind.

pub mod broker;
pub mod dbus;
pub mod preview;

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rmac_storage::Backend as _;

const MANAGED_DIRECTORY: &str = "wallpapers";
const LOCK_FILE: &str = ".portal-import.lock";
const INCOMING_PREFIX: &str = ".incoming-";
const HASH_HEX_BYTES: usize = 64;

static AUTHORITY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consent {
    Accept,
    Decline,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PortalResponse {
    Success = 0,
    Cancelled = 1,
    Other = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Applied { cleanup_pending: bool },
    Cancelled,
}

impl Outcome {
    pub fn response(self) -> PortalResponse {
        match self {
            Self::Applied { .. } => PortalResponse::Success,
            Self::Cancelled => PortalResponse::Cancelled,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    EstablishAuthority,
    RecoverImports,
    AdmitRequest,
    OpenSource,
    StageSource,
    DecodeSource,
    RemoveStaging,
    CreateImport,
    ReadSettings,
    SaveSettings,
    VerifySettings,
    RemoveImport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Portal(rmac_wallpaper::portal::ErrorKind),
    Source(rmac_wallpaper_system::ErrorKind),
    Decode(rmac_wallpaper_image::ErrorKind),
    Io(io::ErrorKind),
    Settings,
    InvalidAuthority,
    AuthorityBusy,
    WrongAuthority,
    ReadbackMismatch,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn response(&self) -> PortalResponse {
        PortalResponse::Other
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("operation", &self.operation)
            .field("kind", &self.kind)
            .field("detail", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "wallpaper portal request failed during {:?} ({:?})",
            self.operation, self.kind
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestId(u64);

struct Staged {
    path: PathBuf,
    fingerprint: rmac_storage::FileFingerprint,
    format: rmac_wallpaper_system::ImageFormat,
}

pub struct Prepared {
    id: RequestId,
    authority: u64,
    app_id: String,
    staged: Option<Staged>,
    image: Arc<rmac_wallpaper_image::Decoded>,
}

impl Prepared {
    pub fn id(&self) -> RequestId {
        self.id
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn image(&self) -> &Arc<rmac_wallpaper_image::Decoded> {
        &self.image
    }

    pub fn byte_len(&self) -> u64 {
        self.staged
            .as_ref()
            .map_or(0, |staged| staged.fingerprint.byte_len)
    }

    pub fn format(&self) -> rmac_wallpaper_system::ImageFormat {
        self.staged
            .as_ref()
            .map_or(rmac_wallpaper_system::ImageFormat::Png, |staged| {
                staged.format
            })
    }
}

impl fmt::Debug for Prepared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Prepared")
            .field("id", &self.id)
            .field("app_id", &self.app_id)
            .field("source", &"<private>")
            .field("byte_len", &self.byte_len())
            .field("format", &self.format())
            .field("width", &self.image.width)
            .field("height", &self.image.height)
            .finish()
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(staged) = self.staged.take() {
            let _ = rmac_storage::remove_file_durable(&staged.path);
        }
    }
}

struct Inner {
    authority: u64,
    managed_root: PathBuf,
    settings_path: PathBuf,
    _lease: File,
    commit: Mutex<()>,
}

#[derive(Clone)]
pub struct Importer {
    inner: Arc<Inner>,
}

impl fmt::Debug for Importer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Importer(<redacted>)")
    }
}

impl Importer {
    pub fn from_environment() -> Result<Self, Error> {
        let settings = rmac_shell_settings::ShellSettingsStore::from_environment()
            .map_err(|error| settings_error(Operation::EstablishAuthority, error))?;
        Self::new(
            managed_root_from_environment()?,
            settings.path().to_path_buf(),
        )
    }

    pub fn new(managed_root: PathBuf, settings_path: PathBuf) -> Result<Self, Error> {
        validate_absolute_path(&managed_root)?;
        validate_absolute_path(&settings_path)?;
        rmac_storage::create_dir_all_private(&managed_root).map_err(|error| {
            io_error(
                Operation::EstablishAuthority,
                error,
                "create the private wallpaper import directory",
            )
        })?;
        let lease = acquire_lease(&managed_root.join(LOCK_FILE))?;
        let authority = next_identity(&AUTHORITY_SEQUENCE, Operation::EstablishAuthority)?;
        let importer = Self {
            inner: Arc::new(Inner {
                authority,
                managed_root,
                settings_path,
                _lease: lease,
                commit: Mutex::new(()),
            }),
        };
        importer.recover_orphans()?;
        Ok(importer)
    }

    /// Validate and decode one local document before any user confirmation.
    /// The returned image is the mandatory preview input. Dropping the request
    /// removes its private staging file as a best-effort safety net.
    pub fn prepare(&self, request: rmac_wallpaper::portal::Request) -> Result<Prepared, Error> {
        let plan = rmac_wallpaper::portal::evaluate(request).map_err(|kind| {
            failure(
                Operation::AdmitRequest,
                ErrorKind::Portal(kind),
                "the request did not satisfy wallpaper portal policy",
            )
        })?;
        let rmac_wallpaper::Source::File(path) = plan.source else {
            return Err(failure(
                Operation::AdmitRequest,
                ErrorKind::InvalidAuthority,
                "portal admission produced a non-file source",
            ));
        };
        let asset = rmac_wallpaper_system::open_file(&path).map_err(|error| {
            failure(
                Operation::OpenSource,
                ErrorKind::Source(error.kind),
                error.detail(),
            )
        })?;
        let format = asset.format;
        let id = next_request_id()?;
        let staging = self.inner.managed_root.join(format!(
            "{INCOMING_PREFIX}{}-{}",
            std::process::id(),
            id.0
        ));
        let mut source = asset.into_file();
        let fingerprint = rmac_storage::FileSystem
            .write_new_private_stream(
                &staging,
                &mut source,
                rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
            )
            .map_err(|error| {
                io_error(
                    Operation::StageSource,
                    error,
                    "stage the validated wallpaper source",
                )
            })?;
        let decoded = (|| {
            let staged_source =
                rmac_wallpaper_system::resolve(&rmac_wallpaper::Source::File(staging.clone()))
                    .map_err(|error| {
                        failure(
                            Operation::OpenSource,
                            ErrorKind::Source(error.kind),
                            error.detail(),
                        )
                    })?;
            rmac_wallpaper_image::Cache::new(0)
                .get_or_decode(
                    staged_source,
                    rmac_compositor::PhysicalSize {
                        width: 1,
                        height: 1,
                    },
                )
                .map_err(|error| {
                    failure(
                        Operation::DecodeSource,
                        ErrorKind::Decode(error.kind),
                        error.detail(),
                    )
                })
        })();
        let image = match decoded {
            Ok(image) => image,
            Err(error) => {
                remove_exact(&staging, Operation::RemoveStaging)?;
                return Err(error);
            }
        };
        Ok(Prepared {
            id,
            authority: self.inner.authority,
            app_id: plan.app_id,
            staged: Some(Staged {
                path: staging,
                fingerprint,
                format,
            }),
            image,
        })
    }

    /// Consume a prepared request exactly once. Decline and cancellation never
    /// read or write shell settings. Acceptance serializes the durable import
    /// and whole-desktop authority mutation with other accepted portal calls.
    pub fn finish(&self, mut prepared: Prepared, consent: Consent) -> Result<Outcome, Error> {
        if prepared.authority != self.inner.authority {
            return Err(failure(
                Operation::AdmitRequest,
                ErrorKind::WrongAuthority,
                "the prepared request belongs to another importer",
            ));
        }
        let staged = prepared.staged.take().ok_or_else(|| {
            failure(
                Operation::AdmitRequest,
                ErrorKind::WrongAuthority,
                "the prepared request no longer owns staged content",
            )
        })?;
        if consent != Consent::Accept {
            remove_exact(&staged.path, Operation::RemoveStaging)?;
            return Ok(Outcome::Cancelled);
        }
        let _commit = self.commit_guard();
        let result = self.commit(&staged);
        if result.is_err() {
            let _ = remove_if_present(&staged.path, Operation::RemoveStaging);
        }
        result
    }

    fn commit(&self, staged: &Staged) -> Result<Outcome, Error> {
        let destination = self
            .inner
            .managed_root
            .join(managed_name(staged.fingerprint, staged.format));
        let created = match std::fs::hard_link(&staged.path, &destination) {
            Ok(()) => {
                if let Err(error) =
                    sync_directory(&self.inner.managed_root, Operation::CreateImport)
                {
                    let _ = remove_exact(&destination, Operation::RemoveImport);
                    return Err(error);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = rmac_storage::fingerprint_bounded_no_follow(
                    &destination,
                    rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
                )
                .map_err(|read_error| {
                    io_error(
                        Operation::CreateImport,
                        read_error,
                        "validate an existing content-addressed import",
                    )
                })?;
                if existing != staged.fingerprint {
                    return Err(failure(
                        Operation::CreateImport,
                        ErrorKind::ReadbackMismatch,
                        "an existing content-addressed import did not match",
                    ));
                }
                false
            }
            Err(error) => {
                return Err(io_error(
                    Operation::CreateImport,
                    error,
                    "create the durable wallpaper import",
                ));
            }
        };
        if let Err(error) = remove_exact(&staged.path, Operation::RemoveStaging) {
            if created {
                let _ = remove_exact(&destination, Operation::RemoveImport);
            }
            return Err(error);
        }
        let imported = match rmac_storage::fingerprint_bounded_no_follow(
            &destination,
            rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
        ) {
            Ok(imported) => imported,
            Err(error) => {
                if created {
                    let _ = remove_exact(&destination, Operation::RemoveImport);
                }
                return Err(io_error(
                    Operation::CreateImport,
                    error,
                    "verify the durable wallpaper import",
                ));
            }
        };
        if imported != staged.fingerprint {
            if created {
                let _ = remove_exact(&destination, Operation::RemoveImport);
            }
            return Err(failure(
                Operation::CreateImport,
                ErrorKind::ReadbackMismatch,
                "the durable import did not match the reviewed source",
            ));
        }
        let result = self.commit_settings(&destination);
        if let Err(error) = &result {
            let definitely_pre_write = error.operation == Operation::ReadSettings;
            if created
                && (definitely_pre_write || !self.destination_may_be_referenced(&destination))
            {
                let _ = remove_exact(&destination, Operation::RemoveImport);
            }
        }
        result?;
        let cleanup_pending = self.remove_unreferenced_imports().is_err();
        Ok(Outcome::Applied { cleanup_pending })
    }

    fn commit_settings(&self, destination: &Path) -> Result<(), Error> {
        let source = destination.to_str().ok_or_else(|| {
            failure(
                Operation::SaveSettings,
                ErrorKind::InvalidAuthority,
                "the managed wallpaper path is not valid UTF-8",
            )
        })?;
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let mut settings = store
            .load()
            .map_err(|error| settings_error(Operation::ReadSettings, error))?
            .settings;
        settings.wallpaper = rmac_shell_settings::WallpaperSettings {
            default: rmac_shell_settings::WallpaperSelection {
                source: Some(source.to_owned()),
                fit: rmac_shell_settings::WallpaperFit::Fill,
            },
            per_output: Default::default(),
        };
        store
            .save(&settings)
            .map_err(|error| settings_error(Operation::SaveSettings, error))?;
        let readback = store
            .load()
            .map_err(|error| settings_error(Operation::VerifySettings, error))?;
        if readback.settings.wallpaper != settings.wallpaper {
            return Err(failure(
                Operation::VerifySettings,
                ErrorKind::ReadbackMismatch,
                "the wallpaper authority changed before readback",
            ));
        }
        Ok(())
    }

    fn recover_orphans(&self) -> Result<usize, Error> {
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let Ok(snapshot) = store.load() else {
            return Ok(0);
        };
        self.remove_unreferenced(&snapshot.settings)
    }

    fn remove_unreferenced_imports(&self) -> Result<usize, Error> {
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let snapshot = store
            .load()
            .map_err(|error| settings_error(Operation::RecoverImports, error))?;
        self.remove_unreferenced(&snapshot.settings)
    }

    fn destination_may_be_referenced(&self, destination: &Path) -> bool {
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let Ok(snapshot) = store.load() else {
            return true;
        };
        referenced_managed_paths(&snapshot.settings.wallpaper, &self.inner.managed_root)
            .contains(destination)
    }

    fn remove_unreferenced(
        &self,
        settings: &rmac_shell_settings::ShellSettings,
    ) -> Result<usize, Error> {
        let referenced = referenced_managed_paths(&settings.wallpaper, &self.inner.managed_root);
        let entries = std::fs::read_dir(&self.inner.managed_root).map_err(|error| {
            io_error(
                Operation::RecoverImports,
                error,
                "enumerate managed wallpaper imports",
            )
        })?;
        let mut removed = 0;
        for entry in entries {
            let entry = entry.map_err(|error| {
                io_error(
                    Operation::RecoverImports,
                    error,
                    "read a managed wallpaper directory entry",
                )
            })?;
            let path = entry.path();
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let removable = name.starts_with(INCOMING_PREFIX)
                || (is_managed_name(name) && !referenced.contains(&path));
            if removable {
                remove_exact(&path, Operation::RecoverImports)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn commit_guard(&self) -> MutexGuard<'_, ()> {
        self.inner
            .commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn managed_root_from_environment() -> Result<PathBuf, Error> {
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

fn acquire_lease(path: &Path) -> Result<File, Error> {
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

fn next_request_id() -> Result<RequestId, Error> {
    next_identity(&REQUEST_SEQUENCE, Operation::AdmitRequest).map(RequestId)
}

fn next_identity(counter: &AtomicU64, operation: Operation) -> Result<u64, Error> {
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

fn validate_absolute_path(path: &Path) -> Result<(), Error> {
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

fn managed_name(
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

fn is_managed_name(name: &str) -> bool {
    let Some((hash, extension)) = name.rsplit_once('.') else {
        return false;
    };
    hash.len() == HASH_HEX_BYTES
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && matches!(extension, "png" | "jpg" | "webp")
}

fn referenced_managed_paths(
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

fn remove_exact(path: &Path, operation: Operation) -> Result<(), Error> {
    rmac_storage::remove_file_durable(path).map_err(|error| {
        io_error(
            operation,
            error,
            "remove a private wallpaper transaction file",
        )
    })
}

fn remove_if_present(path: &Path, operation: Operation) -> Result<(), Error> {
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

fn sync_directory(path: &Path, operation: Operation) -> Result<(), Error> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(operation, error, "sync the wallpaper import directory"))
}

fn io_error(operation: Operation, error: io::Error, detail: &str) -> Error {
    failure(
        operation,
        ErrorKind::Io(error.kind()),
        format!("{detail}: {error}"),
    )
}

fn settings_error(operation: Operation, error: rmac_shell_settings::Error) -> Error {
    failure(operation, ErrorKind::Settings, error.to_string())
}

fn failure(operation: Operation, kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        operation,
        kind,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
