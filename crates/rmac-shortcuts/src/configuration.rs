//! Shortcut backend status and configuration request boundary.

use super::*;

/// Resolve the status snapshot atomically published by the session shortcut
/// broker. This is readable diagnostic authority, not a second binding store.
pub fn backend_status_path() -> Result<PathBuf, Error> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("rmac/shortcuts-status.json"))
        .ok_or_else(|| {
            Error::new(
                Operation::ResolveStatus,
                "XDG_RUNTIME_DIR is not set to an absolute path",
            )
        })
}

pub fn backend_status() -> Result<BackendStatus, Error> {
    backend_status_at(&backend_status_path()?)
}

pub(super) fn backend_status_at(path: &Path) -> Result<BackendStatus, Error> {
    let contents = std::fs::read(path).map_err(|error| {
        Error::new(
            Operation::ReadStatus,
            format!("shortcut broker status is unavailable: {error}"),
        )
    })?;
    serde_json::from_slice(&contents).map_err(|error| {
        Error::new(
            Operation::ParseStatus,
            format!("shortcut broker status is invalid: {error}"),
        )
    })
}

/// Ask the live shortcut broker to open the desktop portal's configuration UI.
///
/// The broker owns the only GlobalShortcuts session. This request/response
/// boundary deliberately does not create another portal session or persist a
/// second copy of the bindings.
pub fn request_shortcut_configuration() -> Result<(), Error> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            Error::new(
                Operation::ResolveControl,
                "XDG_RUNTIME_DIR is not set to an absolute path",
            )
        })?;
    request_shortcut_configuration_at(&runtime, next_control_request_id())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ConfigureRequest {
    pub(super) version: u8,
    pub(super) request_id: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ConfigureOutcome {
    Requested,
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ConfigureResponse {
    pub(super) version: u8,
    pub(super) request_id: u64,
    pub(super) outcome: ConfigureOutcome,
}

pub(super) fn control_socket_path_in(runtime: &Path) -> PathBuf {
    runtime.join("rmac/shortcut-broker-control.sock")
}

pub(super) fn next_control_request_id() -> u64 {
    let sequence = CONTROL_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let clock = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
    (clock.rotate_left(17) ^ sequence ^ u64::from(std::process::id())).max(1)
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn configuration_request_is_coalesced(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    last.is_some_and(|last| now.saturating_duration_since(last) < CONFIGURATION_REQUEST_COOLDOWN)
}

#[cfg(unix)]
pub(super) fn request_shortcut_configuration_at(
    runtime: &Path,
    request_id: u64,
) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;

    let server_path = control_socket_path_in(runtime);
    let parent = server_path.parent().ok_or_else(|| {
        Error::new(
            Operation::ResolveControl,
            "shortcut control socket has no runtime directory",
        )
    })?;
    validate_control_directory(parent, Operation::RequestConfigure)?;
    let client_path = parent.join(format!(
        "shortcut-configure-client-{}-{request_id}.sock",
        std::process::id()
    ));
    let socket = std::os::unix::net::UnixDatagram::bind(&client_path)
        .map_err(|error| Error::new(Operation::RequestConfigure, error.to_string()))?;
    let socket_identity = socket_identity(&client_path, Operation::RequestConfigure)?;
    let _cleanup = DispatchSocketCleanup {
        path: client_path,
        socket_identity,
    };
    std::fs::set_permissions(&_cleanup.path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| Error::new(Operation::RequestConfigure, error.to_string()))?;
    socket
        .set_read_timeout(Some(CONTROL_RESPONSE_TIMEOUT))
        .map_err(|error| Error::new(Operation::RequestConfigure, error.to_string()))?;
    let request = ConfigureRequest {
        version: CONTROL_PROTOCOL_VERSION,
        request_id,
    };
    let bytes = serde_json::to_vec(&request)
        .map_err(|error| Error::new(Operation::RequestConfigure, error.to_string()))?;
    socket
        .send_to(&bytes, &server_path)
        .map_err(|error| Error::new(Operation::RequestConfigure, error.to_string()))?;

    let mut buffer = [0u8; 256];
    let length = socket
        .recv(&mut buffer)
        .map_err(|error| Error::new(Operation::RequestConfigure, error.to_string()))?;
    let response: ConfigureResponse = serde_json::from_slice(&buffer[..length])
        .map_err(|_| Error::new(Operation::RequestConfigure, "broker response was invalid"))?;
    if response.version != CONTROL_PROTOCOL_VERSION || response.request_id != request_id {
        return Err(Error::new(
            Operation::RequestConfigure,
            "broker response did not match this request",
        ));
    }
    match response.outcome {
        ConfigureOutcome::Requested => Ok(()),
        ConfigureOutcome::Unsupported => Err(Error::new(
            Operation::ConfigurePortal,
            "the active GlobalShortcuts portal cannot configure existing shortcuts",
        )),
        ConfigureOutcome::Failed => Err(Error::new(
            Operation::ConfigurePortal,
            "the active GlobalShortcuts portal did not open shortcut configuration",
        )),
    }
}

#[cfg(not(unix))]
pub(super) fn request_shortcut_configuration_at(
    _runtime: &Path,
    _request_id: u64,
) -> Result<(), Error> {
    Err(Error::new(
        Operation::RequestConfigure,
        "shortcut configuration control is available only on Unix",
    ))
}
