use std::fmt;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    pub(crate) detail: String,
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
pub struct RequestId(pub(crate) u64);

pub(crate) struct Staged {
    pub(crate) path: PathBuf,
    pub(crate) fingerprint: rmac_storage::FileFingerprint,
    pub(crate) format: rmac_wallpaper_system::ImageFormat,
}

pub struct Prepared {
    pub(crate) id: RequestId,
    pub(crate) authority: u64,
    pub(crate) app_id: String,
    pub(crate) staged: Option<Staged>,
    pub(crate) image: Arc<rmac_wallpaper_image::Decoded>,
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

pub(crate) struct Inner {
    pub(crate) authority: u64,
    pub(crate) managed_root: PathBuf,
    pub(crate) settings_path: PathBuf,
    pub(crate) _lease: File,
    pub(crate) commit: Mutex<()>,
}
