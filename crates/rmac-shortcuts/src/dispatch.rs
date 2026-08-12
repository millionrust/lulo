//! Authenticated local shortcut dispatch socket authority.

use super::*;

pub fn shortcut_socket_path(id: &ShortcutId) -> Result<PathBuf, Error> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            Error::new(
                Operation::Dispatch,
                "XDG_RUNTIME_DIR is not set to an absolute path",
            )
        })?;
    shortcut_socket_path_in(&runtime, id)
}

pub(super) fn shortcut_socket_path_in(runtime: &Path, id: &ShortcutId) -> Result<PathBuf, Error> {
    if !known_action(id) {
        return Err(Error::new(
            Operation::Dispatch,
            format!("unknown shortcut {}", id.0),
        ));
    }
    Ok(runtime.join(format!("rmac/shortcut-{}.sock", id.0)))
}

pub fn dispatch(id: &ShortcutId) -> Result<(), Error> {
    let specs = default_shortcuts();
    validate_specs(&specs)?;
    if !known_action(id) {
        return Err(Error::new(
            Operation::Dispatch,
            format!("unknown shortcut {}", id.0),
        ));
    }
    if id.0 == "lock" {
        return lock::request().map_err(|error| Error::new(Operation::Dispatch, error.to_string()));
    }
    let path = shortcut_socket_path(id)?;
    let socket = std::os::unix::net::UnixDatagram::unbound()
        .map_err(|error| Error::new(Operation::Dispatch, error.to_string()))?;
    let bytes = serde_json::to_vec(id)
        .map_err(|error| Error::new(Operation::Dispatch, error.to_string()))?;
    socket
        .send_to(&bytes, &path)
        .map_err(|error| Error::new(Operation::Dispatch, error.to_string()))?;
    Ok(())
}

/// Receive dispatcher events for one compiled shell action. Each independently
/// supervised surface owns its own endpoint, so a launcher crash cannot consume
/// or drop Notification Center or Quick Settings activations.
pub async fn watch_dispatches(id: ShortcutId, sender: Sender<Event>) -> Result<(), Error> {
    watch_dispatches_inner(id, sender, None).await
}

/// Receive one surface's dispatches and announce after its socket is bound.
/// Supervised surfaces use this to delay systemd readiness until the first
/// shortcut activation has a live owner.
pub async fn watch_dispatches_ready(
    id: ShortcutId,
    sender: Sender<Event>,
    ready: Sender<()>,
) -> Result<(), Error> {
    watch_dispatches_inner(id, sender, Some(ready)).await
}

pub(super) async fn watch_dispatches_inner(
    id: ShortcutId,
    sender: Sender<Event>,
    ready: Option<Sender<()>>,
) -> Result<(), Error> {
    if !known_action(&id) {
        return Err(Error::new(
            Operation::BindDispatch,
            format!("unknown shortcut {}", id.0),
        ));
    }
    let path = shortcut_socket_path(&id)?;
    watch_dispatches_at(path, id, sender, ready).await
}

pub(super) async fn watch_dispatches_at(
    path: PathBuf,
    id: ShortcutId,
    sender: Sender<Event>,
    ready: Option<Sender<()>>,
) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Error::new(
            Operation::BindDispatch,
            "shortcut socket has no runtime directory",
        )
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|error| Error::new(Operation::BindDispatch, error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};

        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| Error::new(Operation::BindDispatch, error.to_string()))?;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_socket() => {
                let probe = std::os::unix::net::UnixDatagram::unbound()
                    .map_err(|error| Error::new(Operation::BindDispatch, error.to_string()))?;
                match probe.send_to(b"probe", &path) {
                    Ok(_) => {
                        return Err(Error::new(
                            Operation::BindDispatch,
                            "another shortcut consumer already owns this action",
                        ));
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        std::fs::remove_file(&path).map_err(|error| {
                            Error::new(Operation::BindDispatch, error.to_string())
                        })?;
                    }
                    Err(error) => {
                        return Err(Error::new(Operation::BindDispatch, error.to_string()));
                    }
                }
            }
            Ok(_) => {
                return Err(Error::new(
                    Operation::BindDispatch,
                    "shortcut endpoint is not a socket",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::new(Operation::BindDispatch, error.to_string())),
        }
    }

    let socket = async_io::Async::<std::os::unix::net::UnixDatagram>::bind(&path)
        .map_err(|error| Error::new(Operation::BindDispatch, error.to_string()))?;
    #[cfg(unix)]
    let socket_identity = {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| Error::new(Operation::BindDispatch, error.to_string()))?;
        (metadata.dev(), metadata.ino())
    };
    let _cleanup = DispatchSocketCleanup {
        path: path.clone(),
        #[cfg(unix)]
        socket_identity,
    };
    if let Some(ready) = ready {
        ready
            .send(())
            .await
            .map_err(|_| Error::new(Operation::BindDispatch, "readiness receiver stopped"))?;
    }
    let mut sequence = 0u64;
    let mut buffer = [0u8; 512];
    loop {
        let length = socket
            .recv(&mut buffer)
            .await
            .map_err(|error| Error::new(Operation::ReadDispatch, error.to_string()))?;
        let Ok(received) = serde_json::from_slice::<ShortcutId>(&buffer[..length]) else {
            continue;
        };
        if received != id {
            continue;
        }
        sequence = sequence.wrapping_add(1).max(1);
        if sender
            .send(Event::Activated {
                id: received,
                timestamp_ms: sequence,
            })
            .await
            .is_err()
        {
            return Ok(());
        }
    }
}

pub(super) struct DispatchSocketCleanup {
    pub(super) path: PathBuf,
    #[cfg(unix)]
    pub(super) socket_identity: (u64, u64),
}

#[cfg(unix)]
pub(super) fn socket_identity(path: &Path, operation: Operation) -> Result<(u64, u64), Error> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| Error::new(operation, error.to_string()))?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(unix)]
pub(super) fn validate_control_directory(path: &Path, operation: Operation) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| Error::new(operation, error.to_string()))?;
    if !metadata.file_type().is_dir() {
        return Err(Error::new(
            operation,
            "shortcut runtime endpoint parent is not a directory",
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(Error::new(
            operation,
            "shortcut runtime endpoint parent is accessible by other users",
        ));
    }
    Ok(())
}

impl Drop for DispatchSocketCleanup {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let same_socket = std::fs::symlink_metadata(&self.path)
                .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.socket_identity);
            if same_socket {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}
