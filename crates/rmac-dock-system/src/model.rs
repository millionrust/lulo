use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Launch,
    Focus,
    Close,
    UpdatePins,
    Resolve,
    OpenPlace,
    ReviewTrash,
    EmptyTrash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    Io(std::io::ErrorKind),
    Unavailable,
    Transport,
    Protocol,
    Rejected,
    Unsupported,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    pub kind: FailureKind,
    pub detail: String,
}

impl BackendError {
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Launch => "launch application",
            Self::Focus => "focus application window",
            Self::Close => "close application window",
            Self::UpdatePins => "update pinned applications",
            Self::Resolve => "resolve Dock activation",
            Self::OpenPlace => "open Dock place",
            Self::ReviewTrash => "review Empty Trash",
            Self::EmptyTrash => "empty Trash",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: FailureKind,
    pub app_id: String,
    detail: String,
}

impl Error {
    pub(crate) fn new(
        operation: Operation,
        kind: FailureKind,
        app_id: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            kind,
            app_id: app_id.into(),
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not {} {}: {}",
            self.operation, self.app_id, self.detail
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Launched {
        app_id: String,
        launch: rmac_app_launch::Outcome,
    },
    FocusRequested {
        window: rmac_compositor::WindowId,
    },
    CloseRequested {
        window: rmac_compositor::WindowId,
    },
    PinsUpdated {
        pinned: Vec<rmac_shell_settings::AppId>,
    },
    PlaceOpened {
        kind: rmac_dock::SpecialItemKind,
    },
    TrashEmptied {
        remaining_items: usize,
    },
    NoAction,
}

pub trait Backend: Send + Sync + 'static {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>>;

    fn focus_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>>;

    fn close_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>>;

    fn update_pins(
        &self,
        command: &rmac_dock::PinCommand,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>>;

    fn reorder_pins(
        &self,
        reorder: &rmac_dock::drag::RevalidatedReorder,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>>;

    fn open_directory(&self, path: &Path) -> BackendFuture<'_, Result<(), BackendError>>;

    fn open_trash(&self) -> BackendFuture<'_, Result<(), BackendError>>;
}
