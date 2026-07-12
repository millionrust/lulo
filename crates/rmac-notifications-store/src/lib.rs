//! Private, crash-safe Notification Center history and per-app policy.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_notifications::{
    Action, ActionTarget, AppId, BannerPolicy, Content, Delivery, DeliveryPolicy, DisplayHints,
    HistoryPolicy, Indicator, LockScreenVisibility, Notification, NotificationId, Priority, Sound,
    Source, Time,
};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_HISTORY: usize = 500;
const MAX_PER_APP: usize = 100;
const MAX_POLICIES: usize = 512;
const MAX_LOCK_PREVIEWS: usize = 16;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockPreview {
    Show,
    HideContent,
    Hide,
}

/// Bounded, action-free data that a secure session-lock client may render.
///
/// `content=None` intentionally reveals only application identity and recency.
/// Action identifiers, targets, categories, sounds, and replacement identities
/// never cross this boundary.
#[derive(Clone, Eq, PartialEq)]
pub struct LockPreviewRecord {
    pub notification_id: NotificationId,
    pub app_id: AppId,
    pub content: Option<Content>,
    pub updated_at: Time,
}

impl fmt::Debug for LockPreviewRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockPreviewRecord")
            .field("notification_id", &self.notification_id)
            .field("app_id", &"<redacted>")
            .field("content", &self.content.as_ref().map(|_| "<redacted>"))
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AppPolicy {
    pub enabled: bool,
    pub banners: bool,
    pub sounds: bool,
    pub badges: bool,
    pub history: bool,
    pub urgent_through_focus: bool,
    pub lock_preview: LockPreview,
}

impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            banners: true,
            sounds: true,
            badges: true,
            history: true,
            urgent_through_focus: true,
            lock_preview: LockPreview::Hide,
        }
    }
}

impl AppPolicy {
    pub fn delivery(self, focus_active: bool) -> DeliveryPolicy {
        DeliveryPolicy {
            enabled: self.enabled,
            banner: if self.banners {
                BannerPolicy::Allow
            } else {
                BannerPolicy::Suppress
            },
            sounds: self.sounds,
            history: if self.history {
                HistoryPolicy::Allow
            } else {
                HistoryPolicy::Block
            },
            allow_urgent_through_focus: self.urgent_through_focus,
            focus_active,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Recovery {
    #[default]
    None,
    LastGood,
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadSnapshot {
    pub center: Center,
    pub recovery: Recovery,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct Center {
    history: Vec<Notification>,
    policies: BTreeMap<AppId, AppPolicy>,
}

impl fmt::Debug for Center {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Center")
            .field(
                "history",
                &format_args!("<{} redacted records>", self.history.len()),
            )
            .field(
                "policies",
                &format_args!("<{} redacted apps>", self.policies.len()),
            )
            .finish()
    }
}

impl Center {
    pub fn history(&self) -> &[Notification] {
        &self.history
    }

    pub fn policy(&self, app_id: &AppId) -> AppPolicy {
        self.policies.get(app_id).copied().unwrap_or_default()
    }

    pub fn set_policy(&mut self, app_id: AppId, policy: AppPolicy) -> Result<bool, Error> {
        if !self.policies.contains_key(&app_id) && self.policies.len() >= MAX_POLICIES {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        let changed = self.policies.get(&app_id).copied() != Some(policy);
        self.policies.insert(app_id.clone(), policy);
        if !policy.history || !policy.enabled {
            self.clear(Some(&app_id));
        }
        Ok(changed)
    }

    pub fn policies(&self) -> impl Iterator<Item = (&AppId, &AppPolicy)> {
        self.policies.iter()
    }

    pub fn applications(&self) -> Vec<(AppId, AppPolicy)> {
        let app_ids: BTreeSet<_> = self
            .policies
            .keys()
            .chain(self.history.iter().map(|record| record.source.app_id()))
            .cloned()
            .collect();
        app_ids
            .into_iter()
            .map(|app_id| {
                let policy = self.policy(&app_id);
                (app_id, policy)
            })
            .collect()
    }

    /// Project newest unread records through both application hints and the
    /// user's stricter per-app policy. The provider may request fewer records,
    /// but can never exceed the security boundary's fixed maximum.
    pub fn lock_previews(&self, requested: usize) -> Vec<LockPreviewRecord> {
        let limit = requested.min(MAX_LOCK_PREVIEWS);
        self.history
            .iter()
            .rev()
            .filter(|record| record.unread)
            .filter_map(|record| {
                let policy = self.policy(record.source.app_id());
                if !policy.enabled || !policy.history {
                    return None;
                }
                let preview =
                    restrict_lock_preview(policy.lock_preview, record.display.lock_screen);
                (preview != LockPreview::Hide).then(|| LockPreviewRecord {
                    notification_id: record.id,
                    app_id: record.source.app_id().clone(),
                    content: (preview == LockPreview::Show).then(|| record.content.clone()),
                    updated_at: record.updated_at,
                })
            })
            .take(limit)
            .collect()
    }

    pub fn upsert(&mut self, notification: Notification) {
        if !notification.delivery.history {
            return;
        }
        let app_id = notification.source.app_id().clone();
        let policy = self.policy(&app_id);
        if !policy.enabled || !policy.history {
            self.clear(Some(&app_id));
            return;
        }
        self.history
            .retain(|record| record.id != notification.id || record.source != notification.source);
        self.history.push(notification);
        self.enforce_bounds(&app_id);
    }

    pub fn clear(&mut self, app_id: Option<&AppId>) -> bool {
        let before = self.history.len();
        match app_id {
            Some(app_id) => self
                .history
                .retain(|record| record.source.app_id() != app_id),
            None => self.history.clear(),
        }
        self.history.len() != before
    }

    pub fn remove(&mut self, id: NotificationId) -> bool {
        let before = self.history.len();
        self.history.retain(|record| record.id != id);
        self.history.len() != before
    }

    pub fn mark_all_read(&mut self, app_id: Option<&AppId>) -> bool {
        let mut changed = false;
        for record in &mut self.history {
            if app_id.is_none_or(|app_id| record.source.app_id() == app_id) {
                changed |= record.unread;
                record.unread = false;
            }
        }
        changed
    }

    pub fn indicator(&self) -> Indicator {
        let unread = self
            .history
            .iter()
            .filter(|record| record.unread && self.policy(record.source.app_id()).badges);
        Indicator {
            unread_count: unread.clone().count().try_into().unwrap_or(u32::MAX),
            has_urgent: unread
                .into_iter()
                .any(|record| record.priority == Priority::Urgent),
        }
    }

    pub fn groups(&self) -> Vec<Group<'_>> {
        let mut app_ids = BTreeSet::new();
        for record in &self.history {
            app_ids.insert(record.source.app_id());
        }
        let mut groups: Vec<_> = app_ids
            .into_iter()
            .map(|app_id| {
                let mut records: Vec<_> = self
                    .history
                    .iter()
                    .filter(|record| record.source.app_id() == app_id)
                    .collect();
                records.sort_by_key(|record| std::cmp::Reverse(record.updated_at.0));
                Group { app_id, records }
            })
            .collect();
        groups.sort_by_key(|group| {
            std::cmp::Reverse(
                group
                    .records
                    .first()
                    .map(|record| record.updated_at.0)
                    .unwrap_or_default(),
            )
        });
        groups
    }

    fn enforce_bounds(&mut self, app_id: &AppId) {
        while self
            .history
            .iter()
            .filter(|record| record.source.app_id() == app_id)
            .count()
            > MAX_PER_APP
        {
            if let Some(index) = self
                .history
                .iter()
                .position(|record| record.source.app_id() == app_id)
            {
                self.history.remove(index);
            }
        }
        if self.history.len() > MAX_HISTORY {
            self.history.drain(..self.history.len() - MAX_HISTORY);
        }
    }
}

fn restrict_lock_preview(
    policy: LockPreview,
    application_hint: LockScreenVisibility,
) -> LockPreview {
    let hint = match application_hint {
        LockScreenVisibility::Policy | LockScreenVisibility::Show => LockPreview::Show,
        LockScreenVisibility::HideContent => LockPreview::HideContent,
        LockScreenVisibility::Hide => LockPreview::Hide,
    };
    if preview_rank(policy) >= preview_rank(hint) {
        policy
    } else {
        hint
    }
}

fn preview_rank(preview: LockPreview) -> u8 {
    match preview {
        LockPreview::Show => 0,
        LockPreview::HideContent => 1,
        LockPreview::Hide => 2,
    }
}

pub struct Group<'a> {
    pub app_id: &'a AppId,
    pub records: Vec<&'a Notification>,
}

impl fmt::Debug for Group<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Group")
            .field("app_id", &"<redacted>")
            .field(
                "records",
                &format_args!("<{} redacted records>", self.records.len()),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Resolve,
    CreateDirectory,
    Read,
    Parse,
    Validate,
    Serialize,
    Save,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    UnsupportedVersion,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: ErrorKind,
}

impl Error {
    fn new(operation: Operation, kind: ErrorKind) -> Self {
        Self { operation, kind }
    }

    fn io(operation: Operation, error: io::Error) -> Self {
        Self::new(operation, ErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "notification history operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Store(<redacted path>)")
    }
}

impl Store {
    pub fn from_environment() -> Result<Self, Error> {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state"))
            })
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        Ok(Self::at(state_home.join("rmac/notifications/history.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<LoadSnapshot, Error> {
        match self.read_file(&self.path) {
            Ok(center) => Ok(LoadSnapshot {
                center,
                recovery: Recovery::None,
            }),
            Err(error) if error.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                match self.read_file(&self.backup_path()) {
                    Ok(center) => Ok(LoadSnapshot {
                        center,
                        recovery: Recovery::LastGood,
                    }),
                    Err(backup) if backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                        Ok(LoadSnapshot {
                            center: Center::default(),
                            recovery: Recovery::None,
                        })
                    }
                    Err(backup) => Err(backup),
                }
            }
            Err(error) if recoverable(error) => match self.read_file(&self.backup_path()) {
                Ok(center) => Ok(LoadSnapshot {
                    center,
                    recovery: Recovery::LastGood,
                }),
                Err(backup)
                    if recoverable(backup)
                        || backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) =>
                {
                    Ok(LoadSnapshot {
                        center: Center::default(),
                        recovery: Recovery::Empty,
                    })
                }
                Err(backup) => Err(backup),
            },
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, center: &Center) -> Result<(), Error> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        set_private_directory(parent)?;
        let file = StoredFile::from_center(center)?;
        let bytes = serde_json::to_vec(&file)
            .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.backup_path(), &bytes)
            .map_err(|error| Error::io(Operation::Save, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(Operation::Save, error))
    }

    fn read_file(&self, path: &Path) -> Result<Center, Error> {
        let metadata =
            std::fs::metadata(path).map_err(|error| Error::io(Operation::Read, error))?;
        if metadata.len() > MAX_FILE_BYTES || !metadata.is_file() {
            return Err(Error::new(Operation::Read, ErrorKind::Limit));
        }
        let bytes = std::fs::read(path).map_err(|error| Error::io(Operation::Read, error))?;
        let file: StoredFile = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        file.into_center()
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_extension("last-good.json")
    }
}

fn recoverable(error: Error) -> bool {
    matches!(
        error.kind,
        ErrorKind::Invalid | ErrorKind::UnsupportedVersion | ErrorKind::Limit
    )
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::io(Operation::CreateDirectory, error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), Error> {
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct StoredFile {
    version: u32,
    history: Vec<StoredNotification>,
    policies: BTreeMap<String, AppPolicy>,
}

impl StoredFile {
    fn from_center(center: &Center) -> Result<Self, Error> {
        Ok(Self {
            version: VERSION,
            history: center
                .history
                .iter()
                .map(StoredNotification::from_notification)
                .collect(),
            policies: center
                .policies
                .iter()
                .map(|(app_id, policy)| (app_id.as_str().to_owned(), *policy))
                .collect(),
        })
    }

    fn into_center(self) -> Result<Center, Error> {
        if self.version != VERSION {
            return Err(Error::new(
                Operation::Validate,
                ErrorKind::UnsupportedVersion,
            ));
        }
        if self.history.len() > MAX_HISTORY || self.policies.len() > MAX_POLICIES {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        let history = self
            .history
            .into_iter()
            .map(StoredNotification::into_notification)
            .collect::<Result<Vec<_>, _>>()?;
        let policies = self
            .policies
            .into_iter()
            .map(|(app_id, policy)| AppId::parse(app_id).map(|app_id| (app_id, policy)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        let mut center = Center { history, policies };
        let app_ids: Vec<_> = center
            .history
            .iter()
            .map(|record| record.source.app_id().clone())
            .collect();
        for app_id in app_ids {
            center.enforce_bounds(&app_id);
        }
        Ok(center)
    }
}

#[derive(Deserialize, Serialize)]
struct StoredNotification {
    id: u32,
    source: StoredSource,
    title: String,
    body: String,
    priority: StoredPriority,
    default_action: Option<StoredAction>,
    actions: Vec<StoredAction>,
    category: Option<String>,
    display: StoredDisplay,
    created_at: u64,
    updated_at: u64,
    unread: bool,
}

impl StoredNotification {
    fn from_notification(notification: &Notification) -> Self {
        Self {
            id: notification.id.get(),
            source: StoredSource::from_source(&notification.source),
            title: notification.content.title().to_owned(),
            body: notification.content.body().to_owned(),
            priority: notification.priority.into(),
            default_action: notification
                .default_action
                .as_ref()
                .map(StoredAction::from_action),
            actions: notification
                .actions
                .iter()
                .map(StoredAction::from_action)
                .collect(),
            category: notification.category.clone(),
            display: notification.display.into(),
            created_at: notification.created_at.0,
            updated_at: notification.updated_at.0,
            unread: notification.unread,
        }
    }

    fn into_notification(self) -> Result<Notification, Error> {
        let id = NotificationId::from_protocol(self.id)
            .ok_or_else(|| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        let source = self.source.into_source()?;
        let content = Content::new(self.title, self.body)
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        let default_action = self
            .default_action
            .map(StoredAction::into_action)
            .transpose()?;
        let actions = self
            .actions
            .into_iter()
            .map(StoredAction::into_action)
            .collect::<Result<Vec<_>, _>>()?;
        if actions.len() > 8 {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        if self.created_at > self.updated_at
            || self.category.as_ref().is_some_and(|category| {
                category.trim().is_empty()
                    || category.len() > 128
                    || category.chars().any(char::is_control)
            })
        {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        Ok(Notification {
            id,
            source,
            content,
            priority: self.priority.into(),
            default_action,
            actions,
            category: self.category,
            sound: Sound::Silent,
            display: self.display.into(),
            delivery: Delivery {
                banner: false,
                history: true,
                sound: false,
            },
            banner_visible: false,
            created_at: Time(self.created_at),
            updated_at: Time(self.updated_at),
            expires_at: None,
            unread: self.unread,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum StoredSource {
    Portal { app_id: String, external_id: String },
    Freedesktop { app_id: String },
}

impl StoredSource {
    fn from_source(source: &Source) -> Self {
        match source {
            Source::Portal {
                app_id,
                external_id,
            } => Self::Portal {
                app_id: app_id.as_str().to_owned(),
                external_id: external_id.clone(),
            },
            Source::Freedesktop { app_id } => Self::Freedesktop {
                app_id: app_id.as_str().to_owned(),
            },
        }
    }

    fn into_source(self) -> Result<Source, Error> {
        match self {
            Self::Portal {
                app_id,
                external_id,
            } => Source::portal(
                AppId::parse(app_id)
                    .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?,
                external_id,
            )
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid)),
            Self::Freedesktop { app_id } => Ok(Source::Freedesktop {
                app_id: AppId::parse(app_id)
                    .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?,
            }),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct StoredAction {
    id: String,
    label: String,
    target_signature: Option<String>,
    target_bytes: Option<Vec<u8>>,
    purpose: Option<String>,
}

impl StoredAction {
    fn from_action(action: &Action) -> Self {
        Self {
            id: action.id().to_owned(),
            label: action.label().to_owned(),
            target_signature: action.target().map(|target| target.signature().to_owned()),
            target_bytes: action.target().map(|target| target.bytes().to_vec()),
            purpose: action.purpose().map(str::to_owned),
        }
    }

    fn into_action(self) -> Result<Action, Error> {
        let target = match (self.target_signature, self.target_bytes) {
            (None, None) => None,
            (Some(signature), Some(bytes)) => Some(
                ActionTarget::new(signature, bytes)
                    .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?,
            ),
            _ => return Err(Error::new(Operation::Validate, ErrorKind::Invalid)),
        };
        let action = Action::new(self.id, self.label, target)
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        match self.purpose {
            Some(purpose) => action
                .with_purpose(purpose)
                .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid)),
            None => Ok(action),
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum StoredPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl From<Priority> for StoredPriority {
    fn from(priority: Priority) -> Self {
        match priority {
            Priority::Low => Self::Low,
            Priority::Normal => Self::Normal,
            Priority::High => Self::High,
            Priority::Urgent => Self::Urgent,
        }
    }
}

impl From<StoredPriority> for Priority {
    fn from(priority: StoredPriority) -> Self {
        match priority {
            StoredPriority::Low => Self::Low,
            StoredPriority::Normal => Self::Normal,
            StoredPriority::High => Self::High,
            StoredPriority::Urgent => Self::Urgent,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
struct StoredDisplay {
    persistent: bool,
    resident: bool,
    lock_screen: StoredLockVisibility,
}

impl From<DisplayHints> for StoredDisplay {
    fn from(display: DisplayHints) -> Self {
        Self {
            persistent: display.persistent,
            resident: display.resident,
            lock_screen: display.lock_screen.into(),
        }
    }
}

impl From<StoredDisplay> for DisplayHints {
    fn from(display: StoredDisplay) -> Self {
        Self {
            persistent: display.persistent,
            resident: display.resident,
            lock_screen: display.lock_screen.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum StoredLockVisibility {
    Policy,
    Show,
    HideContent,
    Hide,
}

impl From<LockScreenVisibility> for StoredLockVisibility {
    fn from(visibility: LockScreenVisibility) -> Self {
        match visibility {
            LockScreenVisibility::Policy => Self::Policy,
            LockScreenVisibility::Show => Self::Show,
            LockScreenVisibility::HideContent => Self::HideContent,
            LockScreenVisibility::Hide => Self::Hide,
        }
    }
}

impl From<StoredLockVisibility> for LockScreenVisibility {
    fn from(visibility: StoredLockVisibility) -> Self {
        match visibility {
            StoredLockVisibility::Policy => Self::Policy,
            StoredLockVisibility::Show => Self::Show,
            StoredLockVisibility::HideContent => Self::HideContent,
            StoredLockVisibility::Hide => Self::Hide,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::protocol::{self, PortalInput};
    use rmac_notifications::{DeliveryPolicy, Server, TimeoutPolicy};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "rmac-notification-store-{}-{label}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .join("history.json")
    }

    fn notification(app: &str, external: &str, updated: u64, urgent: bool) -> Notification {
        let mut request = protocol::portal(PortalInput {
            app_id: app.into(),
            id: external.into(),
            title: Some(format!("Private title {external}")),
            body: Some("Private body".into()),
            default_action: Some("app.open".into()),
            ..PortalInput::default()
        })
        .unwrap();
        if urgent {
            request.priority = Priority::Urgent;
        }
        let mut server = Server::new(10, TimeoutPolicy::default());
        let outcome = server
            .post(request, Time(updated), DeliveryPolicy::default())
            .unwrap();
        let mut notification = server
            .active()
            .find(|record| record.id == outcome.id)
            .unwrap()
            .clone();
        notification.id = NotificationId::from_protocol(updated as u32).unwrap();
        notification
    }

    #[test]
    fn private_store_round_trips_actions_content_policy_and_mode() {
        let path = temp_path("roundtrip");
        let store = Store::at(path.clone());
        assert!(!format!("{store:?}").contains(path.to_string_lossy().as_ref()));
        let mut center = Center::default();
        let app_id = AppId::parse("org.example.Chat").unwrap();
        center.upsert(notification("org.example.Chat", "one", 10, true));
        center
            .set_policy(
                app_id.clone(),
                AppPolicy {
                    sounds: false,
                    lock_preview: LockPreview::HideContent,
                    ..AppPolicy::default()
                },
            )
            .unwrap();
        store.save(&center).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.recovery, Recovery::None);
        assert_eq!(loaded.center.history().len(), 1);
        assert_eq!(
            loaded.center.history()[0].content.title(),
            "Private title one"
        );
        assert_eq!(
            loaded.center.history()[0]
                .default_action
                .as_ref()
                .unwrap()
                .id(),
            "app.open"
        );
        assert!(!loaded.center.policy(&app_id).sounds);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn lock_projection_applies_the_stricter_hint_without_actions_or_unbounded_data() {
        let mut center = Center::default();
        for (app, preview) in [
            ("org.example.Chat", LockPreview::Show),
            ("org.example.Mail", LockPreview::HideContent),
            ("org.example.AppHint", LockPreview::Show),
            ("org.example.Secret", LockPreview::Hide),
            ("org.example.AppHide", LockPreview::Show),
        ] {
            center
                .set_policy(
                    AppId::parse(app).unwrap(),
                    AppPolicy {
                        lock_preview: preview,
                        ..AppPolicy::default()
                    },
                )
                .unwrap();
        }

        center.upsert(notification("org.example.Chat", "chat", 10, false));
        center.upsert(notification("org.example.Mail", "mail", 20, false));
        let mut app_restricted = notification("org.example.AppHint", "restricted", 30, false);
        app_restricted.display.lock_screen = LockScreenVisibility::HideContent;
        center.upsert(app_restricted);
        center.upsert(notification("org.example.Secret", "secret", 40, false));
        let mut app_hidden = notification("org.example.AppHide", "hidden", 45, false);
        app_hidden.display.lock_screen = LockScreenVisibility::Hide;
        center.upsert(app_hidden);

        let previews = center.lock_previews(usize::MAX);
        assert_eq!(previews.len(), 3);
        assert_eq!(previews[0].app_id.as_str(), "org.example.AppHint");
        assert!(previews[0].content.is_none());
        assert_eq!(previews[1].app_id.as_str(), "org.example.Mail");
        assert!(previews[1].content.is_none());
        assert_eq!(previews[2].app_id.as_str(), "org.example.Chat");
        assert_eq!(
            previews[2].content.as_ref().unwrap().title(),
            "Private title chat"
        );
        let debug = format!("{previews:?}");
        assert!(!debug.contains("org.example"));
        assert!(!debug.contains("Private title"));

        let mail = AppId::parse("org.example.Mail").unwrap();
        center.mark_all_read(Some(&mail));
        assert!(center
            .lock_previews(usize::MAX)
            .iter()
            .all(|preview| preview.app_id.as_str() != mail.as_str()));

        let chat = AppId::parse("org.example.Chat").unwrap();
        for index in 50..80 {
            center.upsert(notification(
                "org.example.Chat",
                &format!("bounded-{index}"),
                index,
                false,
            ));
        }
        assert_eq!(center.lock_previews(usize::MAX).len(), MAX_LOCK_PREVIEWS);
        center
            .set_policy(
                chat,
                AppPolicy {
                    lock_preview: LockPreview::Hide,
                    ..AppPolicy::default()
                },
            )
            .unwrap();
        assert!(center
            .lock_previews(usize::MAX)
            .iter()
            .all(|preview| preview.app_id.as_str() != "org.example.Chat"));
    }

    #[test]
    fn corrupt_primary_recovers_last_good_without_logging_payload() {
        let path = temp_path("recovery");
        let store = Store::at(path.clone());
        let mut center = Center::default();
        center.upsert(notification("org.example.Chat", "one", 10, false));
        store.save(&center).unwrap();
        std::fs::write(&path, b"private corrupt payload").unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.recovery, Recovery::LastGood);
        assert_eq!(loaded.center.history().len(), 1);
        let debug = format!("{:?}", loaded.center);
        assert!(!debug.contains("Private title"));
        assert!(!debug.contains("org.example"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn grouping_clear_read_badges_and_policy_are_coherent() {
        let mut center = Center::default();
        center.upsert(notification("org.example.Chat", "one", 10, false));
        center.upsert(notification("org.example.Mail", "two", 20, true));
        center.upsert(notification("org.example.Chat", "three", 30, false));
        assert_eq!(center.groups().len(), 2);
        assert_eq!(center.groups()[0].app_id.as_str(), "org.example.Chat");
        assert_eq!(
            center.indicator(),
            Indicator {
                unread_count: 3,
                has_urgent: true
            }
        );
        let mail = AppId::parse("org.example.Mail").unwrap();
        center.mark_all_read(Some(&mail));
        assert_eq!(
            center.indicator(),
            Indicator {
                unread_count: 2,
                has_urgent: false
            }
        );
        center
            .set_policy(
                mail.clone(),
                AppPolicy {
                    history: false,
                    ..AppPolicy::default()
                },
            )
            .unwrap();
        assert!(center
            .history()
            .iter()
            .all(|record| record.source.app_id() != &mail));
        center.clear(None);
        assert!(center.history().is_empty());
    }

    #[test]
    fn transient_or_policy_blocked_notifications_never_enter_history() {
        let mut transient = notification("org.example.Chat", "one", 10, false);
        transient.delivery.history = false;
        let mut center = Center::default();
        center.upsert(transient);
        assert!(center.history().is_empty());
        let app_id = AppId::parse("org.example.Chat").unwrap();
        center
            .set_policy(
                app_id,
                AppPolicy {
                    history: false,
                    ..AppPolicy::default()
                },
            )
            .unwrap();
        center.upsert(notification("org.example.Chat", "two", 20, false));
        assert!(center.history().is_empty());
    }

    #[test]
    fn per_app_sound_policy_reaches_authoritative_delivery() {
        let policy = AppPolicy {
            sounds: false,
            ..AppPolicy::default()
        };
        let mut server = Server::new(10, TimeoutPolicy::default());
        let request = protocol::portal(PortalInput {
            app_id: "org.example.Chat".into(),
            id: "one".into(),
            title: Some("Message".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let outcome = server
            .post(request, Time(1), policy.delivery(false))
            .unwrap();
        assert!(outcome.delivery.banner);
        assert!(!outcome.delivery.sound);
        assert!(outcome.delivery.history);
    }

    #[test]
    fn process_scoped_id_reuse_does_not_replace_another_source() {
        let mut center = Center::default();
        let first = notification("org.example.Chat", "one", 10, false);
        let reused = notification("org.example.Mail", "two", 10, false);
        assert_eq!(first.id, reused.id);
        center.upsert(first);
        center.upsert(reused);
        assert_eq!(center.history().len(), 2);

        let replacement = notification("org.example.Chat", "one", 10, true);
        center.upsert(replacement);
        assert_eq!(center.history().len(), 2);
        assert!(center
            .history()
            .iter()
            .any(|record| record.source.app_id().as_str() == "org.example.Mail"));
    }

    #[test]
    fn malformed_and_oversized_files_recover_to_empty_without_content_errors() {
        let path = temp_path("invalid");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
        let loaded = Store::at(path.clone()).load().unwrap();
        assert_eq!(loaded.recovery, Recovery::Empty);
        assert!(loaded.center.history().is_empty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
