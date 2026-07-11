//! Version-aware global shortcut boundary with an explicit niri fallback.

use std::fmt;
use std::path::{Path, PathBuf};

use async_channel::Sender;
use rmac_storage::atomic_write;
use serde::{Deserialize, Serialize};

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
        output.push_str(&format!(
            "    {} repeat=false hotkey-overlay-title=\"{}\" {{ spawn \"{}\" \"{}\"; }}\n",
            shortcut.niri_trigger,
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

pub fn shortcut_socket_path() -> Result<PathBuf, Error> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("rmac/shortcut-events.sock"))
        .ok_or_else(|| {
            Error::new(
                Operation::Dispatch,
                "XDG_RUNTIME_DIR is not set to an absolute path",
            )
        })
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
    let path = shortcut_socket_path()?;
    let socket = std::os::unix::net::UnixDatagram::unbound()
        .map_err(|error| Error::new(Operation::Dispatch, error.to_string()))?;
    let bytes = serde_json::to_vec(id)
        .map_err(|error| Error::new(Operation::Dispatch, error.to_string()))?;
    socket
        .send_to(&bytes, &path)
        .map_err(|error| Error::new(Operation::Dispatch, error.to_string()))?;
    Ok(())
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
    fn dispatcher_rejects_unknown_ids_before_touching_the_runtime_socket() {
        let error = dispatch(&ShortcutId("not-an-rmac-action".into())).unwrap_err();
        assert_eq!(error.operation, Operation::Dispatch);
        assert!(error.detail.contains("unknown shortcut"));
    }
}
