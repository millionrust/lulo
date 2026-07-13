//! Version-aware global shortcut boundary with an explicit niri fallback.

use std::fmt;
use std::path::{Path, PathBuf};

use async_channel::Sender;
use rmac_storage::atomic_write;
use serde::{Deserialize, Serialize};

pub mod lock;
pub mod lock_settings;

pub const PORTAL_MINIMUM_VERSION: u32 = 1;
pub const PORTAL_CONFIGURE_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ShortcutId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShortcutSpec {
    pub id: ShortcutId,
    pub description: String,
    pub preferred_trigger: String,
    pub niri_trigger: String,
}

pub fn default_shortcuts() -> Vec<ShortcutSpec> {
    vec![
        shortcut("launcher", "Open rmac launcher", "LOGO+space", "Mod+Space"),
        shortcut("app-drawer", "Open application drawer", "LOGO+a", "Mod+A"),
        shortcut(
            "notification-center",
            "Open Notification Center",
            "LOGO+n",
            "Mod+N",
        ),
        shortcut(
            "quick-settings",
            "Open Quick Settings",
            "LOGO+CTRL+c",
            "Mod+Ctrl+C",
        ),
        shortcut("lock", "Lock the rmac session", "LOGO+CTRL+q", "Mod+Ctrl+Q"),
    ]
}

fn shortcut(id: &str, description: &str, preferred: &str, niri: &str) -> ShortcutSpec {
    ShortcutSpec {
        id: ShortcutId(id.into()),
        description: description.into(),
        preferred_trigger: preferred.into(),
        niri_trigger: niri.into(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BackendStatus {
    Portal { version: u32, can_configure: bool },
    FallbackRequired { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    Backend { status: BackendStatus },
    Bound { shortcuts: Vec<BoundShortcut> },
    Activated { id: ShortcutId, timestamp_ms: u64 },
    Deactivated { id: ShortcutId, timestamp_ms: u64 },
    BindingsChanged { shortcuts: Vec<BoundShortcut> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundShortcut {
    pub id: ShortcutId,
    pub description: String,
    pub trigger_description: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Validate,
    ConnectPortal,
    BindPortal,
    WatchPortal,
    Dispatch,
    WriteFallback,
    ResolveStatus,
    ReadStatus,
    ParseStatus,
    BindDispatch,
    ReadDispatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub detail: String,
}

impl Error {
    fn new(operation: Operation, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Could not {:?}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

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

fn backend_status_at(path: &Path) -> Result<BackendStatus, Error> {
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

pub fn validate_specs(shortcuts: &[ShortcutSpec]) -> Result<(), Error> {
    let mut ids = std::collections::BTreeSet::new();
    let mut portal_triggers = std::collections::BTreeSet::new();
    let mut niri_triggers = std::collections::BTreeSet::new();
    for shortcut in shortcuts {
        if shortcut.id.0.is_empty()
            || !shortcut.id.0.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
            })
        {
            return Err(Error::new(
                Operation::Validate,
                format!("invalid shortcut id {}", shortcut.id.0),
            ));
        }
        if !ids.insert(&shortcut.id.0) {
            return Err(Error::new(
                Operation::Validate,
                "shortcut IDs must be unique",
            ));
        }
        validate_text(&shortcut.description, "shortcut description")?;
        validate_portal_trigger(&shortcut.preferred_trigger)?;
        validate_niri_trigger(&shortcut.niri_trigger)?;
        if !portal_triggers.insert(&shortcut.preferred_trigger)
            || !niri_triggers.insert(&shortcut.niri_trigger)
        {
            return Err(Error::new(
                Operation::Validate,
                "shortcut triggers must be unique",
            ));
        }
    }
    Ok(())
}

fn validate_text(value: &str, label: &str) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(Error::new(
            Operation::Validate,
            format!("{label} must be non-empty, bounded, and contain no control characters"),
        ))
    } else {
        Ok(())
    }
}

fn validate_portal_trigger(trigger: &str) -> Result<(), Error> {
    let mut parts = trigger.split('+').peekable();
    let mut saw_key = false;
    while let Some(part) = parts.next() {
        if part.is_empty()
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(Error::new(
                Operation::Validate,
                "invalid portal shortcut trigger",
            ));
        }
        if parts.peek().is_none() {
            saw_key = true;
        } else if !matches!(part, "CTRL" | "ALT" | "SHIFT" | "NUM" | "LOGO") {
            return Err(Error::new(
                Operation::Validate,
                format!("unsupported portal modifier {part}"),
            ));
        }
    }
    if saw_key {
        Ok(())
    } else {
        Err(Error::new(Operation::Validate, "portal trigger has no key"))
    }
}

fn validate_niri_trigger(trigger: &str) -> Result<(), Error> {
    if trigger.is_empty()
        || trigger.len() > 128
        || !trigger
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'_'))
    {
        Err(Error::new(
            Operation::Validate,
            "invalid niri shortcut trigger",
        ))
    } else {
        Ok(())
    }
}

pub fn render_niri_fallback(
    shortcuts: &[ShortcutSpec],
    dispatcher: &Path,
) -> Result<String, Error> {
    validate_specs(shortcuts)?;
    if !dispatcher.is_absolute() {
        return Err(Error::new(
            Operation::WriteFallback,
            "dispatcher path must be absolute",
        ));
    }
    let dispatcher = dispatcher
        .to_str()
        .ok_or_else(|| Error::new(Operation::WriteFallback, "dispatcher path must be UTF-8"))?;
    validate_text(dispatcher, "dispatcher path")?;
    let mut output = String::from(
        "// Generated by rmac. Include this file only when the GlobalShortcuts portal is unavailable.\n\n",
    );
    output.push_str("binds {\n");
    for shortcut in shortcuts {
        let allow_when_locked = if shortcut.id.0 == "lock" {
            " allow-when-locked=true"
        } else {
            ""
        };
        output.push_str(&format!(
            "    {} repeat=false{} hotkey-overlay-title=\"{}\" {{ spawn \"{}\" \"{}\"; }}\n",
            shortcut.niri_trigger,
            allow_when_locked,
            escape_kdl(&shortcut.description),
            escape_kdl(dispatcher),
            shortcut.id.0,
        ));
    }
    output.push_str("}\n");
    Ok(output)
}

fn escape_kdl(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn write_niri_fallback(path: &Path, dispatcher: &Path) -> Result<(), Error> {
    let contents = render_niri_fallback(&default_shortcuts(), dispatcher)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(Operation::WriteFallback, "fallback path has no parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| Error::new(Operation::WriteFallback, error.to_string()))?;
    atomic_write(path, contents.as_bytes())
        .map_err(|error| Error::new(Operation::WriteFallback, error.to_string()))
}

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

fn shortcut_socket_path_in(runtime: &Path, id: &ShortcutId) -> Result<PathBuf, Error> {
    if !default_shortcuts()
        .iter()
        .any(|shortcut| shortcut.id == *id)
    {
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
    if !specs.iter().any(|shortcut| shortcut.id == *id) {
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

async fn watch_dispatches_inner(
    id: ShortcutId,
    sender: Sender<Event>,
    ready: Option<Sender<()>>,
) -> Result<(), Error> {
    if !default_shortcuts().iter().any(|shortcut| shortcut.id == id) {
        return Err(Error::new(
            Operation::BindDispatch,
            format!("unknown shortcut {}", id.0),
        ));
    }
    let path = shortcut_socket_path(&id)?;
    watch_dispatches_at(path, id, sender, ready).await
}

async fn watch_dispatches_at(
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

struct DispatchSocketCleanup {
    path: PathBuf,
    #[cfg(unix)]
    socket_identity: (u64, u64),
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
async fn watch_portal(sender: &Sender<Event>) -> Result<(), Error> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::{pin_mut, select, StreamExt as _};

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

    loop {
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
        }
    }
}

#[cfg(target_os = "linux")]
fn convert_bound(shortcut: &ashpd::desktop::global_shortcuts::Shortcut) -> BoundShortcut {
    BoundShortcut {
        id: ShortcutId(shortcut.id().into()),
        description: shortcut.description().into(),
        trigger_description: shortcut.trigger_description().into(),
    }
}

#[cfg(target_os = "linux")]
async fn send(sender: &Sender<Event>, event: Event) -> Result<(), Error> {
    sender
        .send(event)
        .await
        .map_err(|_| Error::new(Operation::WatchPortal, "shortcut consumer closed"))
}

#[cfg(any(target_os = "linux", test))]
fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_unique_standard_and_niri_triggers() {
        let shortcuts = default_shortcuts();
        validate_specs(&shortcuts).unwrap();
        assert_eq!(shortcuts.len(), 5);
        assert!(shortcuts.iter().all(|shortcut| {
            shortcut.preferred_trigger.starts_with("LOGO+")
                && shortcut.niri_trigger.starts_with("Mod+")
        }));
    }

    #[test]
    fn fallback_is_explicit_shell_free_and_uses_one_dispatcher() {
        let output = render_niri_fallback(
            &default_shortcuts(),
            Path::new("/home/test/.local/libexec/rmac/rmac-shortcut-dispatch"),
        )
        .unwrap();
        assert!(output.starts_with("// Generated by rmac"));
        assert_eq!(output.matches("{ spawn ").count(), 5);
        assert!(!output.contains("spawn-sh"));
        assert!(!output.contains("sh -c"));
        assert!(output.contains("repeat=false"));
        assert_eq!(output.matches("allow-when-locked=true").count(), 1);
        assert!(output.contains("Mod+Ctrl+Q repeat=false allow-when-locked=true"));
    }

    #[test]
    fn rejects_duplicate_or_malformed_shortcuts_and_relative_dispatchers() {
        let mut shortcuts = default_shortcuts();
        shortcuts[1].id = shortcuts[0].id.clone();
        assert!(validate_specs(&shortcuts).is_err());
        assert!(render_niri_fallback(&default_shortcuts(), Path::new("relative")).is_err());
    }

    #[test]
    fn duration_conversion_is_bounded() {
        assert_eq!(duration_ms(std::time::Duration::from_millis(42)), 42);
    }

    #[test]
    fn backend_status_snapshot_is_typed_and_rejects_malformed_data() {
        let path = std::env::temp_dir().join(format!(
            "rmac-shortcuts-status-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            br#"{"kind":"portal","version":2,"can_configure":true}"#,
        )
        .unwrap();
        assert_eq!(
            backend_status_at(&path).unwrap(),
            BackendStatus::Portal {
                version: 2,
                can_configure: true,
            }
        );
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(
            backend_status_at(&path).unwrap_err().operation,
            Operation::ParseStatus
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn dispatcher_rejects_unknown_ids_before_touching_the_runtime_socket() {
        let error = dispatch(&ShortcutId("not-an-rmac-action".into())).unwrap_err();
        assert_eq!(error.operation, Operation::Dispatch);
        assert!(error.detail.contains("unknown shortcut"));
    }

    #[test]
    fn dispatcher_endpoints_are_action_scoped() {
        let runtime = Path::new("/tmp/rmac-shortcuts-test");
        let launcher = shortcut_socket_path_in(runtime, &ShortcutId("launcher".into())).unwrap();
        let drawer = shortcut_socket_path_in(runtime, &ShortcutId("app-drawer".into())).unwrap();
        assert_ne!(launcher, drawer);
        assert!(launcher.ends_with("shortcut-launcher.sock"));
        assert!(shortcut_socket_path_in(runtime, &ShortcutId("unknown".into())).is_err());
    }

    #[test]
    fn action_listener_accepts_only_its_typed_dispatch_and_cleans_up() {
        let root = PathBuf::from("/tmp").join(format!(
            "rmac-shortcut-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let path = root.join("shortcut-launcher.sock");
        let id = ShortcutId("launcher".into());
        let (sender, receiver) = async_channel::bounded(2);
        let (ready_sender, ready_receiver) = async_channel::bounded(1);
        async_io::block_on(async {
            let listener =
                watch_dispatches_at(path.clone(), id.clone(), sender, Some(ready_sender));
            let client = async {
                ready_receiver.recv().await.unwrap();
                let socket = std::os::unix::net::UnixDatagram::unbound().unwrap();
                socket.send_to(br#""not-a-shortcut""#, &path).unwrap();
                socket
                    .send_to(&serde_json::to_vec(&id).unwrap(), &path)
                    .unwrap();
                assert_eq!(
                    receiver.recv().await.unwrap(),
                    Event::Activated {
                        id: id.clone(),
                        timestamp_ms: 1,
                    }
                );
                receiver.close();
                socket
                    .send_to(&serde_json::to_vec(&id).unwrap(), &path)
                    .unwrap();
            };
            let (result, ()) = futures_util::join!(listener, client);
            result.unwrap();
        });
        assert!(!path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
