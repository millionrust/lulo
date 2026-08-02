//! Linux GlobalShortcuts portal session and configuration control.

use super::*;

#[cfg(target_os = "linux")]
pub(super) struct ConfigurationControl {
    socket: async_io::Async<std::os::unix::net::UnixDatagram>,
    _cleanup: DispatchSocketCleanup,
}

#[cfg(target_os = "linux")]
pub(super) fn bind_configuration_control() -> Result<ConfigurationControl, Error> {
    use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};

    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            Error::new(
                Operation::ResolveControl,
                "XDG_RUNTIME_DIR is not set to an absolute path",
            )
        })?;
    let path = control_socket_path_in(&runtime);
    let parent = path.parent().ok_or_else(|| {
        Error::new(
            Operation::BindControl,
            "shortcut control socket has no runtime directory",
        )
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|error| Error::new(Operation::BindControl, error.to_string()))?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::new(Operation::BindControl, error.to_string()))?;
    validate_control_directory(parent, Operation::BindControl)?;

    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            let probe = std::os::unix::net::UnixDatagram::unbound()
                .map_err(|error| Error::new(Operation::BindControl, error.to_string()))?;
            match probe.send_to(b"{}", &path) {
                Ok(_) => {
                    return Err(Error::new(
                        Operation::BindControl,
                        "another shortcut broker owns the configuration endpoint",
                    ));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) =>
                {
                    std::fs::remove_file(&path)
                        .map_err(|error| Error::new(Operation::BindControl, error.to_string()))?;
                }
                Err(error) => return Err(Error::new(Operation::BindControl, error.to_string())),
            }
        }
        Ok(_) => {
            return Err(Error::new(
                Operation::BindControl,
                "shortcut configuration endpoint is not a socket",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::new(Operation::BindControl, error.to_string())),
    }

    let socket = async_io::Async::<std::os::unix::net::UnixDatagram>::bind(&path)
        .map_err(|error| Error::new(Operation::BindControl, error.to_string()))?;
    let socket_identity = socket_identity(&path, Operation::BindControl)?;
    let cleanup = DispatchSocketCleanup {
        path,
        socket_identity,
    };
    std::fs::set_permissions(&cleanup.path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| Error::new(Operation::BindControl, error.to_string()))?;
    Ok(ConfigurationControl {
        socket,
        _cleanup: cleanup,
    })
}

#[cfg(target_os = "linux")]
pub(super) async fn receive_configuration_request(
    control: &ConfigurationControl,
) -> Result<Option<(ConfigureRequest, PathBuf)>, Error> {
    let mut buffer = [0u8; 256];
    let (length, address) = control
        .socket
        .recv_from(&mut buffer)
        .await
        .map_err(|error| Error::new(Operation::ReadControl, error.to_string()))?;
    let Some(reply_path) = address.as_pathname().map(Path::to_path_buf) else {
        return Ok(None);
    };
    let Ok(request) = serde_json::from_slice::<ConfigureRequest>(&buffer[..length]) else {
        return Ok(None);
    };
    if request.version != CONTROL_PROTOCOL_VERSION || request.request_id == 0 {
        return Ok(None);
    }
    Ok(Some((request, reply_path)))
}

pub async fn watch(sender: Sender<Event>) -> Result<(), Error> {
    validate_specs(&default_shortcuts())?;
    #[cfg(target_os = "linux")]
    {
        loop {
            if let Err(error) = watch_portal(&sender).await {
                if sender.is_closed() {
                    return Ok(());
                }
                let event = Event::Backend {
                    status: BackendStatus::FallbackRequired {
                        reason: error.detail.clone(),
                    },
                };
                if sender.send(event).await.is_err() {
                    return Ok(());
                }
                async_io::Timer::after(std::time::Duration::from_secs(2)).await;
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = sender
            .send(Event::Backend {
                status: BackendStatus::FallbackRequired {
                    reason: "GlobalShortcuts is available through the XDG portal on Linux.".into(),
                },
            })
            .await;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) async fn watch_portal(sender: &Sender<Event>) -> Result<(), Error> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::{pin_mut, select, FutureExt as _, StreamExt as _};

    let portal = GlobalShortcuts::new()
        .await
        .map_err(|error| Error::new(Operation::ConnectPortal, error.to_string()))?;
    let version = portal
        .get_property::<u32>("version")
        .await
        .map_err(|error| Error::new(Operation::ConnectPortal, error.to_string()))?;
    if version < PORTAL_MINIMUM_VERSION {
        return Err(Error::new(
            Operation::ConnectPortal,
            format!("GlobalShortcuts portal version {version} is unsupported"),
        ));
    }
    let activated = portal
        .receive_activated()
        .await
        .map_err(|error| Error::new(Operation::WatchPortal, error.to_string()))?
        .fuse();
    let deactivated = portal
        .receive_deactivated()
        .await
        .map_err(|error| Error::new(Operation::WatchPortal, error.to_string()))?
        .fuse();
    let changed = portal
        .receive_shortcuts_changed()
        .await
        .map_err(|error| Error::new(Operation::WatchPortal, error.to_string()))?
        .fuse();
    pin_mut!(activated, deactivated, changed);

    let session = portal
        .create_session()
        .await
        .map_err(|error| Error::new(Operation::BindPortal, error.to_string()))?;
    let specs = default_shortcuts();
    let requested: Vec<_> = specs
        .iter()
        .map(|shortcut| {
            NewShortcut::new(&shortcut.id.0, &shortcut.description)
                .preferred_trigger(shortcut.preferred_trigger.as_str())
        })
        .collect();
    let response = portal
        .bind_shortcuts(&session, &requested, None)
        .await
        .and_then(|request| request.response())
        .map_err(|error| Error::new(Operation::BindPortal, error.to_string()))?;
    let configuration_control = bind_configuration_control()?;
    send(
        sender,
        Event::Backend {
            status: BackendStatus::Portal {
                version,
                can_configure: version >= PORTAL_CONFIGURE_VERSION,
            },
        },
    )
    .await?;
    send(
        sender,
        Event::Bound {
            shortcuts: response.shortcuts().iter().map(convert_bound).collect(),
        },
    )
    .await?;

    let mut last_configuration_request: Option<std::time::Instant> = None;
    loop {
        let configuration_request = receive_configuration_request(&configuration_control).fuse();
        pin_mut!(configuration_request);
        select! {
            signal = activated.next() => match signal {
                Some(signal) => {
                    send(sender, Event::Activated {
                        id: ShortcutId(signal.shortcut_id().into()),
                        timestamp_ms: duration_ms(signal.timestamp()),
                    }).await?;
                }
                None => return Err(Error::new(Operation::WatchPortal, "Activated signal stream ended")),
            },
            signal = deactivated.next() => match signal {
                Some(signal) => {
                    send(sender, Event::Deactivated {
                        id: ShortcutId(signal.shortcut_id().into()),
                        timestamp_ms: duration_ms(signal.timestamp()),
                    }).await?;
                }
                None => return Err(Error::new(Operation::WatchPortal, "Deactivated signal stream ended")),
            },
            signal = changed.next() => match signal {
                Some(signal) => {
                    send(sender, Event::BindingsChanged {
                        shortcuts: signal.shortcuts().iter().map(convert_bound).collect(),
                    }).await?;
                }
                None => return Err(Error::new(Operation::WatchPortal, "ShortcutsChanged signal stream ended")),
            },
            received = configuration_request => {
                let Some((request, reply_path)) = received? else {
                    continue;
                };
                let now = std::time::Instant::now();
                let outcome = if version < PORTAL_CONFIGURE_VERSION {
                    ConfigureOutcome::Unsupported
                } else if configuration_request_is_coalesced(last_configuration_request, now) {
                    ConfigureOutcome::Requested
                } else if portal
                    .configure_shortcuts(&session, None, None)
                    .await
                    .is_ok()
                {
                    last_configuration_request = Some(now);
                    ConfigureOutcome::Requested
                } else {
                    ConfigureOutcome::Failed
                };
                let response = ConfigureResponse {
                    version: CONTROL_PROTOCOL_VERSION,
                    request_id: request.request_id,
                    outcome,
                };
                if let Ok(bytes) = serde_json::to_vec(&response) {
                    let _ = configuration_control.socket.send_to(&bytes, &reply_path).await;
                }
            },
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn convert_bound(
    shortcut: &ashpd::desktop::global_shortcuts::Shortcut,
) -> BoundShortcut {
    BoundShortcut {
        id: ShortcutId(shortcut.id().into()),
        description: shortcut.description().into(),
        trigger_description: shortcut.trigger_description().into(),
    }
}

#[cfg(target_os = "linux")]
pub(super) async fn send(sender: &Sender<Event>, event: Event) -> Result<(), Error> {
    sender
        .send(event)
        .await
        .map_err(|_| Error::new(Operation::WatchPortal, "shortcut consumer closed"))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
