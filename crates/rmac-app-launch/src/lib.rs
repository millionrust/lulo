//! Shared, private-safe application and document launch routing.
//!
//! On the supported niri session, argv is sent through direct compositor IPC
//! so niri can attach an XDG activation token to the child. Ordinary desktop
//! sessions and transient niri transport loss retain a shell-free direct-spawn
//! fallback. A compositor rejection is never bypassed.

use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    CompositorActivation,
    DirectFallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Outcome {
    pub process_id: Option<u32>,
    pub delivery: Delivery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(std::io::ErrorKind),
    Rejected,
    Protocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ErrorKind::Io(std::io::ErrorKind::NotFound) => {
                "the application executable is unavailable"
            }
            ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => {
                "permission to start the application was denied"
            }
            ErrorKind::Io(_) => "the application could not be started",
            ErrorKind::Rejected => "the compositor rejected application startup",
            ErrorKind::Protocol => "the compositor could not accept application startup",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemOperation {
    Open,
    Reveal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ItemError {
    pub operation: ItemOperation,
}

impl fmt::Display for ItemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.operation {
            ItemOperation::Open => "the item could not be opened",
            ItemOperation::Reveal => "the item could not be revealed",
        })
    }
}

impl std::error::Error for ItemError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssociationError;

impl fmt::Display for AssociationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("compatible applications could not be loaded")
    }
}

impl std::error::Error for AssociationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenWithError {
    /// The XDG default changed successfully before application startup failed.
    pub default_changed: bool,
}

impl fmt::Display for OpenWithError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.default_changed {
            "the default application changed, but the file could not be opened"
        } else {
            "the file could not be opened with the selected application"
        })
    }
}

impl std::error::Error for OpenWithError {}

/// Start one parsed desktop-entry command without a shell.
pub async fn launch(spec: rmac_apps::LaunchSpec) -> Result<Outcome, Error> {
    if cfg!(target_os = "linux")
        && std::env::var_os(rmac_compositor_niri::SOCKET_PATH_ENV).is_some()
    {
        if let Some(arguments) = rmac_apps::activation_spawn_argv(&spec) {
            if let Ok(command) = rmac_compositor::SpawnCommand::new(arguments) {
                use std::sync::atomic::{AtomicU64, Ordering};

                static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
                let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed).max(1);
                let result = rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                    id: rmac_compositor::ActivationId(id),
                    action: rmac_compositor::Action::Spawn { command },
                })
                .await
                .result;
                match result {
                    Ok(()) => {
                        return Ok(Outcome {
                            process_id: None,
                            delivery: Delivery::CompositorActivation,
                        });
                    }
                    Err(error) if may_fallback(error.kind) => {}
                    Err(error) => {
                        return Err(Error {
                            kind: match error.kind {
                                rmac_compositor::ActionErrorKind::Rejected => ErrorKind::Rejected,
                                _ => ErrorKind::Protocol,
                            },
                        });
                    }
                }
            }
        }
    }

    blocking::unblock(move || {
        rmac_apps::launch(&spec)
            .map(|child| Outcome {
                process_id: Some(child.id()),
                delivery: Delivery::DirectFallback,
            })
            .map_err(|error| Error {
                kind: ErrorKind::Io(error.kind()),
            })
    })
    .await
}

/// Open one local item through the user-mediated desktop boundary.
///
/// The underlying portal may retain a private path and diagnostic, but neither
/// crosses this shared application boundary.
pub async fn open_item(path: PathBuf) -> Result<(), ItemError> {
    rmac_portal::open_item(&path).await.map_err(|_| ItemError {
        operation: ItemOperation::Open,
    })
}

/// Reveal one local item in the platform file manager.
///
/// Callers receive only the operation class so a private path or portal detail
/// cannot accidentally enter a launcher, notification, or application UI.
pub async fn reveal_item(path: PathBuf) -> Result<(), ItemError> {
    rmac_portal::show_item(&path).await.map_err(|_| ItemError {
        operation: ItemOperation::Reveal,
    })
}

/// Reveal the trusted source for one catalog application.
pub async fn reveal_application(application: rmac_apps::Application) -> Result<(), ItemError> {
    reveal_item(application.source).await
}

/// Resolve current XDG MIME handlers away from the UI executor.
pub async fn file_association(
    path: PathBuf,
) -> Result<rmac_apps::FileAssociation, AssociationError> {
    blocking::unblock(move || rmac_apps::file_association(&path))
        .await
        .map_err(|_| AssociationError)
}

/// Revalidate and open one file with an exact compatible desktop application.
///
/// The catalog authority still distinguishes a successfully retained default
/// change from a later launch failure. All private command and path details are
/// reduced before returning to application UI.
pub async fn open_file_with(
    path: PathBuf,
    expected_mime_type: String,
    application_id: String,
    make_default: bool,
) -> Result<(), OpenWithError> {
    blocking::unblock(move || {
        rmac_apps::open_file_with(&path, &expected_mime_type, &application_id, make_default)
    })
    .await
    .map_err(|error| OpenWithError {
        default_changed: error.default_changed,
    })
}

fn may_fallback(kind: rmac_compositor::ActionErrorKind) -> bool {
    matches!(
        kind,
        rmac_compositor::ActionErrorKind::Unavailable
            | rmac_compositor::ActionErrorKind::Transport
            | rmac_compositor::ActionErrorKind::Unsupported
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_actionable_and_redact_commands() {
        let error = Error {
            kind: ErrorKind::Io(std::io::ErrorKind::NotFound),
        };
        assert_eq!(
            error.to_string(),
            "the application executable is unavailable"
        );
        assert!(!format!("{error:?}").contains("/home"));
    }

    #[test]
    fn fallback_never_bypasses_compositor_rejection_or_protocol_failure() {
        assert!(may_fallback(rmac_compositor::ActionErrorKind::Unavailable));
        assert!(may_fallback(rmac_compositor::ActionErrorKind::Transport));
        assert!(may_fallback(rmac_compositor::ActionErrorKind::Unsupported));
        assert!(!may_fallback(rmac_compositor::ActionErrorKind::Rejected));
        assert!(!may_fallback(rmac_compositor::ActionErrorKind::Protocol));
    }

    #[test]
    fn integration_errors_are_private_and_actionable() {
        let open = ItemError {
            operation: ItemOperation::Open,
        };
        let reveal = ItemError {
            operation: ItemOperation::Reveal,
        };
        assert_eq!(open.to_string(), "the item could not be opened");
        assert_eq!(reveal.to_string(), "the item could not be revealed");
        assert_eq!(
            AssociationError.to_string(),
            "compatible applications could not be loaded"
        );
        assert_eq!(
            OpenWithError {
                default_changed: true,
            }
            .to_string(),
            "the default application changed, but the file could not be opened"
        );
        let diagnostics = format!(
            "{open:?} {reveal:?} {:?} {:?}",
            AssociationError,
            OpenWithError {
                default_changed: false,
            }
        );
        assert!(!diagnostics.contains("/home"));
        assert!(!diagnostics.contains("gio"));
    }
}
