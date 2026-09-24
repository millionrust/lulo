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
    Restore,
    Reveal,
    Terminate,
    UpdatePins,
    UpdateStacks,
    Hide,
    ShowAllWindows,
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

/// The notice shown when an application the Dock launched cannot start,
/// worded as the Mac words it: a title naming the app and a reason.
pub fn launch_failure_notice(name: &str, kind: FailureKind) -> (String, String) {
    let reason = match kind {
        FailureKind::Io(std::io::ErrorKind::NotFound) => {
            "Its program is missing. Reinstall the application and try again."
        }
        FailureKind::Io(std::io::ErrorKind::PermissionDenied) => {
            "You don’t have permission to run its program."
        }
        FailureKind::Rejected => "The desktop refused to start it.",
        FailureKind::Unavailable | FailureKind::Transport => {
            "The desktop could not be reached. Try again in a moment."
        }
        _ => "It could not be started.",
    };
    (
        format!("The application “{name}” can’t be opened."),
        reason.to_owned(),
    )
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
            Self::Restore => "restore minimized window",
            Self::Reveal => "show application in Files",
            Self::Terminate => "terminate application",
            Self::UpdatePins => "update pinned applications",
            Self::UpdateStacks => "update Dock stacks",
            Self::Resolve => "resolve Dock activation",
            Self::OpenPlace => "open Dock place",
            Self::ReviewTrash => "review Empty Trash",
            Self::EmptyTrash => "empty Trash",
            Self::Hide => "hide application windows",
            Self::ShowAllWindows => "show all application windows",
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
    RestoreRequested {
        window: rmac_compositor::WindowId,
    },
    ApplicationRevealed {
        app_id: String,
    },
    TerminationRequested {
        app_id: String,
        kind: rmac_dock::TerminationKind,
    },
    PinsUpdated {
        pinned: Vec<rmac_shell_settings::AppId>,
    },
    StacksUpdated {
        stacks: Vec<rmac_shell_settings::DockStackEntry>,
    },
    HideRequested {
        app_id: String,
        windows: usize,
    },
    ShowAllWindowsRequested {
        app_id: String,
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

    /// Move a parked window back to the workspace it was minimized from.
    fn restore_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        let _ = (request_id, window);
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unsupported,
                "restoring minimized windows is unavailable",
            ))
        })
    }

    fn reveal_application(&self, source: &Path) -> BackendFuture<'_, Result<(), BackendError>> {
        let _ = source;
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unsupported,
                "revealing applications is unavailable",
            ))
        })
    }

    fn terminate_application(
        &self,
        pids: &[u32],
        kind: rmac_dock::TerminationKind,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        let _ = (pids, kind);
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unsupported,
                "application termination is unavailable",
            ))
        })
    }

    fn update_pins(
        &self,
        command: &rmac_dock::PinCommand,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>>;

    /// Add, remove, or reconfigure a folder/file stack. Unsupported by
    /// default so an existing `Backend` implementor need not stub it.
    fn update_stacks(
        &self,
        command: &rmac_dock::StackCommand,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::DockStackEntry>, BackendError>> {
        let _ = command;
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unsupported,
                "Dock stacks are unavailable",
            ))
        })
    }

    /// Hide windows by parking them, recording each origin workspace so Show
    /// All (and a Dock click) can bring them back.
    fn hide_windows(
        &self,
        request_id: rmac_compositor::ActivationId,
        windows: &[rmac_compositor::WindowId],
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        let _ = (request_id, windows);
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unsupported,
                "hiding applications is unavailable",
            ))
        })
    }

    /// Bring `window` forward, then open App Exposé for its application.
    fn show_all_windows(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        let _ = (request_id, window);
        Box::pin(async {
            Err(BackendError::new(
                FailureKind::Unsupported,
                "App Exposé is unavailable",
            ))
        })
    }

    fn reorder_pins(
        &self,
        reorder: &rmac_dock::drag::RevalidatedReorder,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>>;

    fn open_directory(&self, path: &Path) -> BackendFuture<'_, Result<(), BackendError>>;

    fn open_trash(&self) -> BackendFuture<'_, Result<(), BackendError>>;
}
