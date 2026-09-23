//! Display-only origin of a history record: when it arrived and which
//! installed application sent it.
//!
//! Legacy `Notify` callers are identified by their unique bus name, which
//! means nothing to a person. Notification Center shows the sending app's
//! real name and icon the way macOS does, so the service records the
//! kernel-reported origin of the sending process: its systemd XDG app scope
//! (the desktop-entry ID a launcher put it in) and its executable. The
//! unauthenticated `app_name` argument is never used. None of this feeds
//! policy, replacement or authorization; it only labels and stacks cards.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_notifications::NotificationId;

/// `(id, posted_unix_ms, desktop_id, executable)`; `0` and `""` mean unknown.
pub type WireOrigin = (u32, u64, String, String);

pub const MAX_WIRE_ORIGINS: usize = 500;
const MAX_ORIGINS: usize = 1_024;
const MAX_DESKTOP_ID_BYTES: usize = 255;
const MAX_EXECUTABLE_BYTES: usize = 4_096;
/// A sender annotation may briefly precede the history upsert it belongs to.
const UNCLAIMED_GRACE_MS: u64 = 60_000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Origin {
    pub posted_unix_ms: Option<u64>,
    pub desktop_id: Option<String>,
    pub executable: Option<String>,
}

impl Origin {
    pub fn from_wire(posted_unix_ms: u64, desktop_id: String, executable: String) -> Self {
        Self {
            posted_unix_ms: (posted_unix_ms != 0).then_some(posted_unix_ms),
            desktop_id: Some(desktop_id).filter(|id| valid_desktop_id(id)),
            executable: Some(executable).filter(|path| valid_executable(path)),
        }
    }
}

/// In-memory origins for the current service run. Records restored from
/// disk after a restart have no origin and show no time or process identity.
#[derive(Clone, Default)]
pub struct Origins {
    inner: Arc<Mutex<BTreeMap<NotificationId, Origin>>>,
}

impl std::fmt::Debug for Origins {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Origins(<redacted>)")
    }
}

impl Origins {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<NotificationId, Origin>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records the sending process of a legacy `Notify` call.
    pub fn sender(
        &self,
        id: NotificationId,
        desktop_id: Option<String>,
        executable: Option<String>,
        now_ms: u64,
    ) {
        let mut origins = self.lock();
        if origins.len() >= MAX_ORIGINS && !origins.contains_key(&id) {
            return;
        }
        origins.insert(
            id,
            Origin {
                posted_unix_ms: Some(now_ms),
                desktop_id: desktop_id.filter(|id| valid_desktop_id(id)),
                executable: executable.filter(|path| valid_executable(path)),
            },
        );
    }

    /// Stamps the arrival time of any record entering history, keeping a
    /// sender annotation that is already present.
    pub fn posted(&self, id: NotificationId, now_ms: u64) {
        let mut origins = self.lock();
        if origins.len() >= MAX_ORIGINS && !origins.contains_key(&id) {
            return;
        }
        origins.entry(id).or_default().posted_unix_ms = Some(now_ms);
    }

    /// Drops origins whose record left history, except fresh annotations
    /// whose history upsert has not been applied yet.
    pub fn retain(&self, live: &[NotificationId], now_ms: u64) {
        self.lock().retain(|id, origin| {
            live.contains(id)
                || origin
                    .posted_unix_ms
                    .is_some_and(|posted| now_ms.saturating_sub(posted) < UNCLAIMED_GRACE_MS)
        });
    }

    pub fn wire(&self, live: &[NotificationId]) -> Vec<WireOrigin> {
        let origins = self.lock();
        live.iter()
            .filter_map(|id| {
                let origin = origins.get(id)?;
                Some((
                    id.get(),
                    origin.posted_unix_ms.unwrap_or_default(),
                    origin.desktop_id.clone().unwrap_or_default(),
                    origin.executable.clone().unwrap_or_default(),
                ))
            })
            .take(MAX_WIRE_ORIGINS)
            .collect()
    }
}

pub fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn valid_desktop_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DESKTOP_ID_BYTES
        && !id.starts_with('.')
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn valid_executable(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= MAX_EXECUTABLE_BYTES
        && !path.chars().any(char::is_control)
}

/// The desktop-entry ID in a systemd XDG application unit, as launchers
/// name them: `app[-<launcher>]-<id>-<random>.scope` or
/// `app[-<launcher>]-<id>[@<random>].service`, with `-` inside the ID
/// escaped as `\x2d`.
pub fn desktop_id_from_cgroup(contents: &str) -> Option<String> {
    let path = contents
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .or_else(|| contents.lines().last()?.rsplit(':').next())?;
    let unit = path.trim().rsplit('/').find(|segment| {
        segment.starts_with("app-")
            && (segment.ends_with(".scope") || segment.ends_with(".service"))
    })?;
    let parts: Vec<&str> = if let Some(scope) = unit.strip_suffix(".scope") {
        let mut parts: Vec<&str> = scope.strip_prefix("app-")?.split('-').collect();
        // The trailing random part is mandatory for scopes.
        if parts.len() < 2 {
            return None;
        }
        parts.pop();
        parts
    } else {
        let service = unit.strip_suffix(".service")?.strip_prefix("app-")?;
        let service = service.split_once('@').map_or(service, |(name, _)| name);
        service.split('-').collect()
    };
    let escaped = match parts.as_slice() {
        [id] | [_, id] => *id,
        _ => return None,
    };
    let id = escaped.replace("\\x2d", "-");
    valid_desktop_id(&id).then_some(id)
}

fn executable_from_link(link: &std::path::Path) -> Option<String> {
    let path = link.to_str()?;
    let path = path.strip_suffix(" (deleted)").unwrap_or(path);
    valid_executable(path).then(|| path.to_owned())
}

/// Reads the origin of a live process. Missing or unreadable `/proc` entries
/// simply yield no origin.
pub fn process_origin(pid: u32) -> (Option<String>, Option<String>) {
    if pid == 0 {
        return (None, None);
    }
    let desktop_id = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .ok()
        .and_then(|contents| desktop_id_from_cgroup(&contents));
    let executable = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|link| executable_from_link(&link));
    (desktop_id, executable)
}

/// Resolves the process behind a unique bus name through the bus daemon.
pub async fn sender_origin(
    connection: &zbus::Connection,
    sender: &str,
) -> (Option<String>, Option<String>) {
    let Ok(name) = zbus::names::BusName::try_from(sender) else {
        return (None, None);
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(connection).await else {
        return (None, None);
    };
    let Ok(pid) = proxy.get_connection_unix_process_id(name).await else {
        return (None, None);
    };
    blocking::unblock(move || process_origin(pid)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u32) -> NotificationId {
        NotificationId::from_protocol(value).unwrap()
    }

    #[test]
    fn cgroup_scopes_and_services_yield_exact_desktop_ids() {
        let scope = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/\
                     app-gnome-org.gnome.Nautilus-4242.scope\n";
        assert_eq!(
            desktop_id_from_cgroup(scope).as_deref(),
            Some("org.gnome.Nautilus")
        );
        let flatpak = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/\
                       app-flatpak-org.mozilla.firefox-99.scope";
        assert_eq!(
            desktop_id_from_cgroup(flatpak).as_deref(),
            Some("org.mozilla.firefox")
        );
        let service = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/\
                       app-org.example.Chat@a1b2.service";
        assert_eq!(
            desktop_id_from_cgroup(service).as_deref(),
            Some("org.example.Chat")
        );
        let escaped = "0::/app.slice/app-niri-my\\x2dtool-12.scope";
        assert_eq!(desktop_id_from_cgroup(escaped).as_deref(), Some("my-tool"));
    }

    #[test]
    fn non_application_cgroups_yield_nothing() {
        assert_eq!(
            desktop_id_from_cgroup("0::/user.slice/user-1000.slice/session-2.scope"),
            None
        );
        assert_eq!(
            desktop_id_from_cgroup("0::/user.slice/user@1000.service/app.slice/dbus.service"),
            None
        );
        assert_eq!(
            desktop_id_from_cgroup("0::/app.slice/app-a-b-c-1.scope"),
            None
        );
        assert_eq!(desktop_id_from_cgroup("0::/app.slice/app-1.scope"), None);
        assert_eq!(desktop_id_from_cgroup(""), None);
    }

    #[test]
    fn executables_must_be_absolute_and_drop_the_deleted_marker() {
        assert_eq!(
            executable_from_link(std::path::Path::new("/usr/bin/tool (deleted)")).as_deref(),
            Some("/usr/bin/tool")
        );
        assert_eq!(executable_from_link(std::path::Path::new("tool")), None);
    }

    #[test]
    fn origins_follow_history_and_keep_fresh_annotations() {
        let origins = Origins::default();
        origins.sender(id(1), Some("org.example.Chat".into()), None, 1_000);
        origins.posted(id(1), 1_500);
        origins.posted(id(2), 2_000);
        assert_eq!(
            origins.wire(&[id(1), id(2)]),
            vec![
                (1, 1_500, "org.example.Chat".into(), String::new()),
                (2, 2_000, String::new(), String::new()),
            ]
        );

        origins.sender(id(3), None, Some("/usr/bin/tool".into()), 2_000);
        origins.retain(&[id(1)], 10_000);
        assert_eq!(origins.wire(&[id(2), id(3)]).len(), 2);
        origins.retain(&[id(1)], 2_000 + UNCLAIMED_GRACE_MS);
        assert_eq!(origins.wire(&[id(2), id(3)]), Vec::new());
        assert_eq!(origins.wire(&[id(1)]).len(), 1);
    }

    #[test]
    fn wire_origins_decode_unknowns_and_reject_invalid_text() {
        assert_eq!(
            Origin::from_wire(0, String::new(), String::new()),
            Origin::default()
        );
        let origin = Origin::from_wire(5, "bad id".into(), "relative".into());
        assert_eq!(origin.posted_unix_ms, Some(5));
        assert_eq!(origin.desktop_id, None);
        assert_eq!(origin.executable, None);
    }
}
