//! Shared, private-safe application launch routing.
//!
//! On the supported niri session, argv is sent through direct compositor IPC
//! so niri can attach an XDG activation token to the child. Ordinary desktop
//! sessions and transient niri transport loss retain a shell-free direct-spawn
//! fallback. A compositor rejection is never bypassed.

use std::fmt;

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
}
