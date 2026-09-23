use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDateTime};
use gpui::SharedString;
use rmac_notifications::NotificationId;
use rmac_notifications_linux::center::{ActionSelection, HistoryRecord};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ApplicationIdentity {
    pub(crate) name: SharedString,
    pub(crate) icon: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Busy {
    /// Clearing every record of one displayed group (its key).
    ClearGroup(String),
    // Wired once the notification options menu lands (§5.3).
    #[allow(dead_code)]
    DisableApp(String),
    Remove(NotificationId),
    Invoke(NotificationId, ActionSelection),
}

/// One installed application as Notification Center needs it.
#[derive(Clone, Debug)]
pub(crate) struct CatalogEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) icon: Option<PathBuf>,
    /// The canonical executable its `Exec=` program resolves to, if any.
    pub(crate) executable: Option<PathBuf>,
}

/// Exact lookups from a notification's origin to an installed application.
/// Desktop IDs match exactly (or without the `.desktop` suffix); an
/// executable matches only when exactly one application launches it
/// directly, so shells, interpreters and launch wrappers never label a card.
#[derive(Clone, Default)]
pub(crate) struct ApplicationCatalog {
    by_id: BTreeMap<String, (String, ApplicationIdentity)>,
    by_executable: BTreeMap<PathBuf, (String, ApplicationIdentity)>,
    /// Exact display names owned by exactly one application; the freedesktop
    /// `app_name` of most senders (e.g. `notify-send -a Files`).
    by_name: BTreeMap<String, (String, ApplicationIdentity)>,
}

/// Programs that start other programs; their executable says nothing about
/// which application sent a notification.
const LAUNCHER_PROGRAMS: &[&str] = &[
    "bash",
    "dash",
    "dbus-run-session",
    "electron",
    "env",
    "fish",
    "flatpak",
    "gjs",
    "gtk-launch",
    "java",
    "mono",
    "node",
    "nodejs",
    "perl",
    "pkexec",
    "python",
    "ruby",
    "sh",
    "snap",
    "sudo",
    "systemd-run",
    "wine",
    "xdg-open",
    "zsh",
];

fn is_launcher_program(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    let base =
        name.trim_end_matches(|character: char| character.is_ascii_digit() || character == '.');
    LAUNCHER_PROGRAMS.contains(&base)
}

impl ApplicationCatalog {
    pub(crate) fn new(entries: Vec<CatalogEntry>) -> Self {
        let mut by_id = BTreeMap::new();
        let mut executables = BTreeMap::<PathBuf, Vec<(String, ApplicationIdentity)>>::new();
        let mut names = BTreeMap::<String, Vec<(String, ApplicationIdentity)>>::new();
        for entry in entries {
            let name = entry.name.clone();
            let identity = ApplicationIdentity {
                name: entry.name.into(),
                icon: entry.icon,
            };
            names
                .entry(name)
                .or_default()
                .push((entry.id.clone(), identity.clone()));
            by_id.insert(entry.id.clone(), (entry.id.clone(), identity.clone()));
            if let Some(alias) = entry.id.strip_suffix(".desktop") {
                by_id
                    .entry(alias.to_owned())
                    .or_insert_with(|| (entry.id.clone(), identity.clone()));
            }
            if let Some(executable) = entry.executable.filter(|path| !is_launcher_program(path)) {
                executables
                    .entry(executable)
                    .or_default()
                    .push((entry.id, identity));
            }
        }
        let by_executable = executables
            .into_iter()
            .filter_map(|(path, mut owners)| (owners.len() == 1).then(|| (path, owners.remove(0))))
            .collect();
        let by_name = names
            .into_iter()
            .filter_map(|(name, mut owners)| (owners.len() == 1).then(|| (name, owners.remove(0))))
            .collect();
        Self {
            by_id,
            by_executable,
            by_name,
        }
    }

    fn by_id(&self, id: &str) -> Option<&(String, ApplicationIdentity)> {
        self.by_id.get(id)
    }

    /// The group key and visible identity of a record's sending application.
    pub(crate) fn resolve(&self, record: &HistoryRecord) -> (String, ApplicationIdentity) {
        let origin = &record.origin;
        let known = origin
            .desktop_id
            .as_deref()
            .and_then(|id| self.by_id(id))
            .or_else(|| self.by_id(&record.app_id))
            .or_else(|| {
                origin
                    .executable
                    .as_deref()
                    .and_then(|path| self.by_executable.get(Path::new(path)))
            })
            .or_else(|| self.by_name.get(&record.app_id));
        if let Some((id, identity)) = known {
            return (id.clone(), identity.clone());
        }
        if let Some(id) = &origin.desktop_id {
            return (
                id.clone(),
                ApplicationIdentity {
                    name: id.clone().into(),
                    icon: None,
                },
            );
        }
        if let Some(path) = &origin.executable {
            let name = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned();
            return (
                format!("exe:{path}"),
                ApplicationIdentity {
                    name: name.into(),
                    icon: None,
                },
            );
        }
        (
            format!("app:{}", record.app_id),
            ApplicationIdentity {
                name: fallback_app_name(&record.app_id).into(),
                icon: None,
            },
        )
    }

    pub(crate) fn identity(&self, app_id: &str) -> ApplicationIdentity {
        self.by_id(app_id)
            .map(|(_, identity)| identity.clone())
            .unwrap_or_else(|| ApplicationIdentity {
                name: fallback_app_name(app_id).into(),
                icon: None,
            })
    }
}

/// Resolves an `Exec=` program to its canonical executable, searching
/// `PATH` for bare names.
pub(crate) fn resolve_program(program: &str) -> Option<PathBuf> {
    let program = Path::new(program);
    if program.is_absolute() {
        return std::fs::canonicalize(program).ok();
    }
    if program.components().count() != 1 {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| std::fs::canonicalize(candidate).ok())
}

pub(crate) fn catalog_entries(catalog: Vec<rmac_apps::Application>) -> Vec<CatalogEntry> {
    catalog
        .into_iter()
        .map(|application| {
            let executable = match &application.launch {
                rmac_apps::LaunchSpec::Command { program, .. } => resolve_program(program),
                rmac_apps::LaunchSpec::OpenPath(_) => None,
            };
            CatalogEntry {
                id: application.id,
                name: application.name,
                icon: application.icon,
                executable,
            }
        })
        .collect()
}

pub(crate) fn fallback_app_name(app_id: &str) -> String {
    if app_id.is_empty() || app_id.starts_with(':') || app_id.starts_with(".1.") {
        "Application".into()
    } else {
        app_id.to_owned()
    }
}

/// Records of one sending application, newest first, as macOS stacks them.
pub(crate) struct RecordGroup<'a> {
    pub(crate) key: String,
    pub(crate) identity: ApplicationIdentity,
    /// Service application IDs whose records make up this group.
    pub(crate) app_ids: Vec<String>,
    pub(crate) records: Vec<&'a HistoryRecord>,
}

impl RecordGroup<'_> {
    fn newest(&self) -> u64 {
        self.records
            .iter()
            .filter_map(|record| record.origin.posted_unix_ms)
            .max()
            .unwrap_or_default()
    }
}

/// Groups records by resolved application. Several senders of one app (for
/// example each `notify-send` process) share one stack. Records and groups
/// are newest first; records without a known time keep the service order
/// after timed ones.
pub(crate) fn group_records<'a>(
    records: &'a [HistoryRecord],
    catalog: &ApplicationCatalog,
) -> Vec<RecordGroup<'a>> {
    let mut positions = BTreeMap::<String, usize>::new();
    let mut groups: Vec<RecordGroup<'a>> = Vec::new();
    for record in records {
        let (key, identity) = catalog.resolve(record);
        let position = *positions.entry(key.clone()).or_insert_with(|| {
            groups.push(RecordGroup {
                key,
                identity,
                app_ids: Vec::new(),
                records: Vec::new(),
            });
            groups.len() - 1
        });
        let group = &mut groups[position];
        if !group.app_ids.contains(&record.app_id) {
            group.app_ids.push(record.app_id.clone());
        }
        group.records.push(record);
    }
    for group in &mut groups {
        group.records.sort_by_key(|record| {
            std::cmp::Reverse(record.origin.posted_unix_ms.unwrap_or_default())
        });
    }
    groups.sort_by_key(|group| std::cmp::Reverse(group.newest()));
    groups
}

/// Surface width: the measured backdrop spans the right 420 pt.
pub(crate) const PANEL_WIDTH: f32 = rmac_notifications_linux::center_surface::LOGICAL_WIDTH as f32;
const MAX_PANEL_HEIGHT: f32 = rmac_notifications_linux::center_surface::LOGICAL_HEIGHT as f32;
/// First card 8 pt under the menu bar; room below the last for its shadow.
pub(crate) const COLUMN_TOP: f32 = 8.0;
pub(crate) const COLUMN_BOTTOM: f32 = 16.0;
/// The backdrop dim fades out 182 pt under the bar, so the surface is never
/// shorter than that.
pub(crate) const DIM_HEIGHT: f32 = 182.0;

/// The layer-surface height for a column of cards `content` points tall.
pub(crate) fn panel_height(content: f32) -> f32 {
    let content = if content.is_finite() {
        content.max(0.0)
    } else {
        MAX_PANEL_HEIGHT
    };
    (COLUMN_TOP + content + COLUMN_BOTTOM).clamp(DIM_HEIGHT, MAX_PANEL_HEIGHT)
}

fn local_time(unix_ms: u64, offset_seconds: i32) -> Option<NaiveDateTime> {
    let utc = DateTime::from_timestamp_millis(i64::try_from(unix_ms).ok()?)?.naive_utc();
    utc.checked_add_signed(Duration::seconds(i64::from(offset_seconds)))
}

/// The macOS relative timestamp on a notification card: "now", "5m ago",
/// "2h ago", "Yesterday", a weekday within the week, then a short date.
pub(crate) fn relative_time(posted_unix_ms: u64, now_unix_ms: u64, offset_seconds: i32) -> String {
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let elapsed = now_unix_ms.saturating_sub(posted_unix_ms);
    if elapsed < MINUTE {
        return "now".into();
    }
    if elapsed < HOUR {
        return format!("{}m ago", elapsed / MINUTE);
    }
    if elapsed < DAY {
        return format!("{}h ago", elapsed / HOUR);
    }
    let (Some(posted), Some(now)) = (
        local_time(posted_unix_ms, offset_seconds),
        local_time(now_unix_ms, offset_seconds),
    ) else {
        return String::new();
    };
    let days = (now.date() - posted.date()).num_days();
    if days <= 1 {
        "Yesterday".into()
    } else if days < 7 {
        posted.format("%A").to_string()
    } else if posted.format("%Y").to_string() == now.format("%Y").to_string() {
        posted.format("%-d %b").to_string()
    } else {
        posted.format("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::{Content, Priority};
    use rmac_notifications_linux::origin::Origin;

    fn record(id: u32, app_id: &str, origin: Origin) -> HistoryRecord {
        HistoryRecord {
            id: NotificationId::from_protocol(id).unwrap(),
            app_id: app_id.to_owned(),
            content: Content::new("Title", "Body").unwrap(),
            priority: Priority::Normal,
            unread: false,
            actions: Vec::new(),
            origin,
        }
    }

    fn origin(posted: Option<u64>, desktop_id: Option<&str>, executable: Option<&str>) -> Origin {
        Origin {
            posted_unix_ms: posted,
            desktop_id: desktop_id.map(Into::into),
            executable: executable.map(Into::into),
        }
    }

    fn entry(id: &str, name: &str, executable: Option<&str>) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            name: name.into(),
            icon: Some(PathBuf::from(format!("/icons/{id}.svg"))),
            executable: executable.map(PathBuf::from),
        }
    }

    fn catalog() -> ApplicationCatalog {
        ApplicationCatalog::new(vec![
            entry("org.example.Chat.desktop", "Chat", Some("/usr/bin/chat")),
            entry("org.example.Files.desktop", "Files", Some("/usr/bin/files")),
            entry(
                "org.example.Script.desktop",
                "Script",
                Some("/usr/bin/python3.12"),
            ),
            entry("org.example.A.desktop", "A", Some("/usr/bin/shared")),
            entry("org.example.B.desktop", "B", Some("/usr/bin/shared")),
        ])
    }

    #[test]
    fn unresolved_application_identity_is_never_guessed() {
        assert_eq!(
            fallback_app_name("org.example.Private.desktop"),
            "org.example.Private.desktop"
        );
        assert_eq!(fallback_app_name(""), "Application");
        assert_eq!(fallback_app_name(":1.42"), "Application");
        assert_eq!(fallback_app_name(".1.42"), "Application");
    }

    #[test]
    fn origins_resolve_to_real_names_and_icons_exactly() {
        let catalog = catalog();
        let (key, identity) = catalog.resolve(&record(
            1,
            ":1.9",
            origin(None, Some("org.example.Chat"), None),
        ));
        assert_eq!(key, "org.example.Chat.desktop");
        assert_eq!(identity.name.as_ref(), "Chat");
        assert!(identity.icon.is_some());

        let (key, identity) = catalog.resolve(&record(
            2,
            ":1.10",
            origin(None, None, Some("/usr/bin/files")),
        ));
        assert_eq!(key, "org.example.Files.desktop");
        assert_eq!(identity.name.as_ref(), "Files");

        let (_, identity) = catalog.resolve(&record(3, "org.example.Chat", Origin::default()));
        assert_eq!(identity.name.as_ref(), "Chat");

        // `notify-send -a Files` names the app rather than its desktop id.
        let (key, identity) = catalog.resolve(&record(4, "Files", Origin::default()));
        assert_eq!(key, "org.example.Files.desktop");
        assert_eq!(identity.name.as_ref(), "Files");
    }

    #[test]
    fn interpreters_and_shared_executables_never_label_a_card() {
        let catalog = catalog();
        let (key, identity) = catalog.resolve(&record(
            1,
            ":1.4",
            origin(None, None, Some("/usr/bin/python3.12")),
        ));
        assert_eq!(key, "exe:/usr/bin/python3.12");
        assert_eq!(identity.name.as_ref(), "python3.12");
        assert!(identity.icon.is_none());

        let (key, _) = catalog.resolve(&record(
            2,
            ":1.5",
            origin(None, None, Some("/usr/bin/shared")),
        ));
        assert_eq!(key, "exe:/usr/bin/shared");

        let (key, identity) = catalog.resolve(&record(3, ":1.6", Origin::default()));
        assert_eq!(key, "app::1.6");
        assert_eq!(identity.name.as_ref(), "Application");
    }

    #[test]
    fn senders_of_one_app_share_a_stack_newest_first() {
        let catalog = catalog();
        let records = vec![
            record(
                1,
                ":1.1",
                origin(Some(1_000), Some("org.example.Chat"), None),
            ),
            record(2, ":1.2", origin(Some(5_000), None, Some("/usr/bin/files"))),
            record(
                3,
                ":1.3",
                origin(Some(3_000), Some("org.example.Chat"), None),
            ),
            record(4, ":1.4", Origin::default()),
        ];
        let groups = group_records(&records, &catalog);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].identity.name.as_ref(), "Files");
        assert_eq!(groups[1].identity.name.as_ref(), "Chat");
        assert_eq!(
            groups[1].app_ids,
            vec![":1.1".to_owned(), ":1.3".to_owned()]
        );
        let ids: Vec<u32> = groups[1]
            .records
            .iter()
            .map(|record| record.id.get())
            .collect();
        assert_eq!(ids, vec![3, 1]);
        assert_eq!(groups[2].key, "app::1.4");
    }

    #[test]
    fn the_surface_hugs_its_cards_between_the_dim_and_the_cap() {
        assert_eq!(panel_height(0.0), DIM_HEIGHT);
        assert_eq!(panel_height(64.0), DIM_HEIGHT);
        assert_eq!(panel_height(300.0), 324.0);
        assert_eq!(panel_height(10_000.0), MAX_PANEL_HEIGHT);
        assert_eq!(panel_height(f32::MAX), MAX_PANEL_HEIGHT);
        assert_eq!(panel_height(f32::NAN), MAX_PANEL_HEIGHT);
        assert_eq!(PANEL_WIDTH, 420.0);
    }

    #[test]
    fn timestamps_read_like_macos() {
        // 2026-09-23 12:30:00 UTC, a Wednesday.
        let now = 1_790_166_600_000_u64;
        let minute = 60_000;
        let hour = 60 * minute;
        let day = 24 * hour;
        assert_eq!(relative_time(now, now, 0), "now");
        assert_eq!(relative_time(now - 59_000, now, 0), "now");
        assert_eq!(relative_time(now + 5_000, now, 0), "now");
        assert_eq!(relative_time(now - 5 * minute, now, 0), "5m ago");
        assert_eq!(relative_time(now - 2 * hour - minute, now, 0), "2h ago");
        assert_eq!(relative_time(now - day - hour, now, 0), "Yesterday");
        assert_eq!(relative_time(now - 3 * day, now, 0), "Sunday");
        assert_eq!(relative_time(now - 10 * day, now, 0), "13 Sep");
        assert_eq!(relative_time(now - 400 * day, now, 0), "19 Aug 2025");
        // 13:00 local two days ago still reads as a weekday.
        assert_eq!(relative_time(now - 2 * day, now, 5 * 3_600), "Monday");
    }
}
