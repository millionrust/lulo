use crate::{Delivery, Error, ErrorKind, Outcome};

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

pub(crate) fn may_fallback(kind: rmac_compositor::ActionErrorKind) -> bool {
    matches!(
        kind,
        rmac_compositor::ActionErrorKind::Unavailable
            | rmac_compositor::ActionErrorKind::Transport
            | rmac_compositor::ActionErrorKind::Unsupported
    )
}
