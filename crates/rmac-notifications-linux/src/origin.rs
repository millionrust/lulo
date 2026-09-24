//! Display-only origin of a history record: when it arrived and which
//! installed application sent it.
//!
//! Legacy `Notify` callers are identified by their unique bus name, which
//! means nothing to a person. Notification Center shows the sending app's
//! real name, icon and a relative time the way macOS does. The service
//! records:
//! - the kernel-reported origin of the sending process: its systemd XDG app
//!   scope (the desktop-entry ID a launcher put it in) and its executable;
//! - what the sender says about itself: the `desktop-entry` hint, `app_name`,
//!   and its icon (the `image-path` hint, else `app_icon`), as the
//!   freedesktop specification intends them to be used.
//!
//! None of this feeds policy, replacement or authorization; it only labels,
//! times and stacks cards. Once a record enters history its origin is kept
//! in the history file (`rmac_notifications_store::Label`), so a restart
//! keeps each card's time, name and icon.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_notifications::NotificationId;

/// `(id, posted_unix_ms, desktop_id, executable, hinted_desktop_id,
/// app_name, icon)`; `0` and `""` mean unknown.
pub type WireOrigin = (u32, u64, String, String, String, String, String);

/// A record's origin is the history store's display label.
pub type Origin = rmac_notifications_store::Label;

pub const MAX_WIRE_ORIGINS: usize = 500;
const MAX_ORIGINS: usize = 1_024;
/// A sender annotation may briefly precede the history upsert it belongs to.
const UNCLAIMED_GRACE_MS: u64 = 60_000;

/// What a legacy `Notify` call says about its sender, beside the
/// kernel-reported process origin.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SenderHints {
    pub desktop_entry: Option<String>,
    pub app_name: Option<String>,
    /// `image-path` when given, else `app_icon`.
    pub icon: Option<String>,
}

/// Icons are looked up at twice the 32 pt card icon, sharp on a 2× screen.
const SENDER_ICON_PIXELS: u32 = 64;

/// Turns a sender's icon (a theme name, an absolute path or a `file://`
/// URI) into a file path the Center and banners can draw, or `None` when it
/// names nothing that exists. Blocking: it reads the icon theme, so call it
/// off the UI thread.
pub fn resolve_icon_hint(icon: &str) -> Option<String> {
    static THEMES: std::sync::OnceLock<Mutex<rmac_apps::ThemedIconResolver>> =
        std::sync::OnceLock::new();
    let path = if icon.starts_with('/') {
        std::path::PathBuf::from(icon)
    } else if icon.starts_with("file://") {
        url::Url::parse(icon).ok()?.to_file_path().ok()?
    } else {
        THEMES
            .get_or_init(|| Mutex::new(rmac_apps::ThemedIconResolver::current()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .resolve(icon, SENDER_ICON_PIXELS)?
    };
    path.is_file()
        .then(|| path.to_str().map(str::to_owned))
        .flatten()
}

/// Decodes one wire origin. Unknown (`0`, `""`) and invalid fields are
/// dropped.
pub fn from_wire(
    posted_unix_ms: u64,
    desktop_id: String,
    executable: String,
    hinted_desktop_id: String,
    app_name: String,
    icon: String,
) -> Origin {
    let known = |value: String| Some(value).filter(|value| !value.is_empty());
    Origin {
        posted_unix_ms: Some(posted_unix_ms),
        desktop_id: known(desktop_id),
        executable: known(executable),
        hinted_desktop_id: known(hinted_desktop_id),
        app_name: known(app_name),
        icon: known(icon),
    }
    .sanitized()
}

pub fn to_wire(id: NotificationId, origin: &Origin) -> WireOrigin {
    (
        id.get(),
        origin.posted_unix_ms.unwrap_or_default(),
        origin.desktop_id.clone().unwrap_or_default(),
        origin.executable.clone().unwrap_or_default(),
        origin.hinted_desktop_id.clone().unwrap_or_default(),
        origin.app_name.clone().unwrap_or_default(),
        origin.icon.clone().unwrap_or_default(),
    )
}

/// In-memory origins of notifications not yet (or never) in history, such
/// as a banner whose app keeps no history. History keeps its own copy.
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

    /// Records the sending process of a legacy `Notify` call and what it
    /// said about itself.
    pub fn sender(
        &self,
        id: NotificationId,
        desktop_id: Option<String>,
        executable: Option<String>,
        hints: SenderHints,
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
                desktop_id,
                executable,
                hinted_desktop_id: hints.desktop_entry,
                app_name: hints.app_name,
                icon: hints.icon,
            }
            .sanitized(),
        );
    }

    /// Stamps the arrival time of any record entering history, keeping a
    /// sender annotation that is already present, and returns the origin.
    pub fn posted(&self, id: NotificationId, now_ms: u64) -> Origin {
        let mut origins = self.lock();
        if origins.len() >= MAX_ORIGINS && !origins.contains_key(&id) {
            return Origin {
                posted_unix_ms: Some(now_ms),
                ..Origin::default()
            };
        }
        let origin = origins.entry(id).or_default();
        origin.posted_unix_ms = Some(now_ms);
        origin.clone()
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

    /// The origin recorded for one notification, if any.
    pub fn get(&self, id: NotificationId) -> Option<Origin> {
        self.lock().get(&id).cloned()
    }
}

pub fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}

use rmac_notifications_store::{valid_desktop_id, valid_executable};

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
        origins.sender(
            id(1),
            Some("org.example.Chat".into()),
            None,
            SenderHints::default(),
            1_000,
        );
        assert_eq!(
            origins.posted(id(1), 1_500).desktop_id.as_deref(),
            Some("org.example.Chat")
        );
        assert_eq!(origins.posted(id(2), 2_000).posted_unix_ms, Some(2_000));
        assert_eq!(origins.get(id(1)).unwrap().posted_unix_ms, Some(1_500));

        origins.sender(
            id(3),
            None,
            Some("/usr/bin/tool".into()),
            SenderHints::default(),
            2_000,
        );
        origins.retain(&[id(1)], 10_000);
        assert!(origins.get(id(2)).is_some());
        assert!(origins.get(id(3)).is_some());
        origins.retain(&[id(1)], 2_000 + UNCLAIMED_GRACE_MS);
        assert!(origins.get(id(2)).is_none());
        assert!(origins.get(id(3)).is_none());
        assert!(origins.get(id(1)).is_some());
    }

    #[test]
    fn a_sender_names_its_app_and_icon_but_invalid_claims_are_dropped() {
        let origins = Origins::default();
        origins.sender(
            id(1),
            None,
            None,
            SenderHints {
                desktop_entry: Some("org.rmac.TextEditor.desktop".into()),
                app_name: Some("Text Editor".into()),
                icon: Some("org.rmac.TextEditor".into()),
            },
            1_000,
        );
        let origin = origins.get(id(1)).unwrap();
        assert_eq!(
            origin.hinted_desktop_id.as_deref(),
            Some("org.rmac.TextEditor")
        );
        assert_eq!(origin.app_name.as_deref(), Some("Text Editor"));
        assert_eq!(origin.icon.as_deref(), Some("org.rmac.TextEditor"));

        origins.sender(
            id(2),
            None,
            None,
            SenderHints {
                desktop_entry: Some("../evil".into()),
                app_name: Some("line\nbreak".into()),
                icon: Some("relative/icon.png".into()),
            },
            1_000,
        );
        let origin = origins.get(id(2)).unwrap();
        assert_eq!(origin.hinted_desktop_id, None);
        assert_eq!(origin.app_name, None);
        assert_eq!(origin.icon, None);
    }

    #[test]
    fn wire_origins_round_trip_and_decode_unknowns() {
        assert_eq!(
            from_wire(
                0,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new()
            ),
            Origin::default()
        );
        let origin = from_wire(
            5,
            "bad id".into(),
            "relative".into(),
            "org.example.Chat".into(),
            "Chat".into(),
            "file:///usr/share/icons/chat.png".into(),
        );
        assert_eq!(origin.posted_unix_ms, Some(5));
        assert_eq!(origin.desktop_id, None);
        assert_eq!(origin.executable, None);
        let wire = to_wire(id(7), &origin);
        let (_, posted, desktop_id, executable, hinted, name, icon) = wire;
        assert_eq!(
            from_wire(posted, desktop_id, executable, hinted, name, icon),
            origin
        );
    }
}
