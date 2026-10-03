use crate::{Delivery, Error, ErrorKind, Outcome};

/// Start one parsed desktop-entry command without a shell.
pub async fn launch(spec: rmac_apps::LaunchSpec) -> Result<Outcome, Error> {
    launch_with_color_scheme(spec, None).await
}

/// Pass a fresh resolved scheme to compositor-spawned apps. Resident launchers
/// can outlive the scheme niri inherited when the desktop session started.
pub async fn launch_with_color_scheme(
    spec: rmac_apps::LaunchSpec,
    color_scheme: Option<&str>,
) -> Result<Outcome, Error> {
    let color_scheme = color_scheme
        .filter(|scheme| matches!(*scheme, "dark" | "light"))
        .map(str::to_owned);
    if cfg!(target_os = "linux")
        && std::env::var_os(rmac_compositor_niri::SOCKET_PATH_ENV).is_some()
    {
        if let Some(mut arguments) = rmac_apps::activation_spawn_argv(&spec) {
            // niri acknowledges `spawn` before it forks, and a failed exec is
            // only logged in niri's own output, so a missing or unrunnable
            // program would otherwise "launch" with nothing on screen.
            if let Some(program) = arguments.first().cloned() {
                let path = std::env::var_os("PATH");
                blocking::unblock(move || {
                    runnable_program(std::path::Path::new(&program), path.as_deref())
                })
                .await
                .map_err(|kind| Error {
                    kind: ErrorKind::Io(kind),
                })?;
            }
            arguments = with_color_scheme(arguments, color_scheme.as_deref());
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
        let child = match (
            color_scheme.as_deref(),
            rmac_apps::activation_spawn_argv(&spec),
        ) {
            (Some(scheme), Some(arguments)) if cfg!(target_os = "linux") => {
                std::process::Command::new("/usr/bin/env")
                    .arg(format!("RMAC_COLOR_SCHEME={scheme}"))
                    .args(arguments)
                    .spawn()
            }
            _ => rmac_apps::launch(&spec),
        };
        child
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

fn with_color_scheme(mut arguments: Vec<String>, scheme: Option<&str>) -> Vec<String> {
    if let Some(scheme) = scheme {
        arguments.splice(
            0..0,
            ["/usr/bin/env".into(), format!("RMAC_COLOR_SCHEME={scheme}")],
        );
    }
    arguments
}

#[cfg(test)]
mod color_scheme_tests {
    use super::with_color_scheme;

    #[test]
    fn compositor_spawn_receives_the_live_scheme() {
        assert_eq!(
            with_color_scheme(vec!["rmac-system-settings".into()], Some("dark")),
            [
                "/usr/bin/env",
                "RMAC_COLOR_SCHEME=dark",
                "rmac-system-settings"
            ]
        );
    }
}

pub(crate) fn may_fallback(kind: rmac_compositor::ActionErrorKind) -> bool {
    matches!(
        kind,
        rmac_compositor::ActionErrorKind::Unavailable
            | rmac_compositor::ActionErrorKind::Transport
            | rmac_compositor::ActionErrorKind::Unsupported
    )
}

/// Whether `program` names an executable the way `execvp` would find it:
/// a path containing `/` is used as is, a bare name is searched in `path`.
pub(crate) fn runnable_program(
    program: &std::path::Path,
    path: Option<&std::ffi::OsStr>,
) -> Result<(), std::io::ErrorKind> {
    if program.as_os_str().is_empty() {
        return Err(std::io::ErrorKind::NotFound);
    }
    if program.components().count() > 1 || program.is_absolute() {
        return executable(program);
    }
    let mut denied = false;
    for directory in std::env::split_paths(path.unwrap_or_default()) {
        let directory = if directory.as_os_str().is_empty() {
            std::path::PathBuf::from(".")
        } else {
            directory
        };
        match executable(&directory.join(program)) {
            Ok(()) => return Ok(()),
            Err(std::io::ErrorKind::PermissionDenied) => denied = true,
            Err(_) => {}
        }
    }
    Err(if denied {
        std::io::ErrorKind::PermissionDenied
    } else {
        std::io::ErrorKind::NotFound
    })
}

fn executable(candidate: &std::path::Path) -> Result<(), std::io::ErrorKind> {
    // Follows symlinks, so a dangling link counts as missing.
    let metadata = std::fs::metadata(candidate).map_err(|error| error.kind())?;
    if !metadata.is_file() {
        return Err(std::io::ErrorKind::NotFound);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c_path) = std::ffi::CString::new(candidate.as_os_str().as_bytes()) else {
            return Err(std::io::ErrorKind::NotFound);
        };
        // SAFETY: `c_path` is a valid NUL-terminated path for this call.
        if unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } != 0 {
            return Err(std::io::ErrorKind::PermissionDenied);
        }
    }
    Ok(())
}
